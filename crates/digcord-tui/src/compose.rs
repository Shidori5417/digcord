//! Yazma ve mesaj eylemleri: INSERT / SELECT / COMMAND modları, tepki istemi, silme onayı,
//! otomatik tamamlama ve "yazıyor" bildirimi kısıtlaması.

use std::time::{Duration, Instant};

use digcord_model::{Permissions, ReactionEmoji, Snowflake};
use digcord_store::State;
use ratatui::crossterm::event::{KeyCode, KeyEvent};

use crate::editor::EditOutcome;
use crate::keymap::{Act, Scope};
use crate::{Action, App, Focus, Scroll};

/// Resmi istemci "yazıyor"u yaklaşık bu aralıkla yeniler (PROTOCOL_NOTES §4, doğrulanmadı).
const TYPING_EVERY: Duration = Duration::from_secs(8);
/// Tamamlama penceresindeki en fazla aday.
pub(crate) const MAX_CANDIDATES: usize = 6;
/// Ücretsiz hesap yükleme sınırı (doğrulanmadı; Discord 2024'te 10 MB'a düşürdü).
const MAX_UPLOAD: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Normal,
    Insert,
    Select,
    Command,
    React,
    ConfirmDelete(Snowflake),
    /// `Ctrl+K` hızlı geçiş.
    Switcher,
    /// Tam ekran resim görüntüleyici.
    Viewer,
    /// `:settings` tam ekran ayarlar.
    Settings,
    /// `:search` / `:pins` tam ekran sonuç listesi.
    Search,
}

/// Yazılan metnin hedefi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Target {
    #[default]
    New,
    Reply(Snowflake),
    Edit(Snowflake),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub label: String,
    pub insert: String,
}

impl App {
    /// NORMAL moddan diğer modlara geçiş eylemleri; işlenmezse `None`.
    pub(crate) fn normal_mode_key(&mut self, act: Act) -> Option<Option<Action>> {
        match act {
            Act::InsertMode if self.open.is_some() => {
                self.enter_insert(crate::compose::Target::New);
                Some(None)
            }
            Act::SelectMode if self.open.is_some() => {
                self.enter_select();
                Some(None)
            }
            Act::CommandMode => {
                self.command.clear();
                self.mode = Mode::Command;
                Some(None)
            }
            // Açık kanal yokken `i`/`v` sessizce yutulur.
            Act::InsertMode | Act::SelectMode => Some(None),
            _ => None,
        }
    }

    fn can_send(&self) -> bool {
        self.open
            .is_some_and(|o| self.shared.read().can_send(o.channel))
    }

    fn enter_insert(&mut self, target: Target) {
        if self
            .open
            .is_some_and(|o| self.shared.read().is_detached(o.channel))
        {
            self.notice = Some("Eski mesajlardasın; yazmak için önce G ile güncele dön".into());
            return;
        }
        if !self.can_send() {
            let locked = self
                .open
                .is_some_and(|o| self.shared.read().is_thread_locked(o.channel));
            self.notice = Some(if locked {
                "Bu thread kilitli; yazamazsın".into()
            } else {
                "Bu kanala yazma iznin yok".into()
            });
            return;
        }
        if let Target::Edit(id) = target {
            let text = self.open.and_then(|o| {
                self.shared
                    .read()
                    .find_message(o.channel, id)
                    .map(|m| m.content.clone())
            });
            self.editor.set_text(&text.unwrap_or_default());
        }
        self.target = target;
        self.comp_sel = 0;
        self.mode = Mode::Insert;
        self.focus = Focus::Messages;
    }

    pub(crate) fn enter_select(&mut self) {
        let Some(open) = self.open else { return };
        let st = self.shared.read();
        let Some(msgs) = st.messages(open.channel).filter(|m| !m.is_empty()) else {
            return;
        };
        // Görünen en alttaki mesajdan başla.
        let idx = crate::view::anchor_index(msgs, self.scroll.anchor);
        self.selected = Some(msgs[idx].id);
        drop(st);
        self.mode = Mode::Select;
        self.focus = Focus::Messages;
        self.pending_d = false;
    }

    pub(crate) fn insert_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Some(open) = self.open else {
            self.mode = Mode::Normal;
            return None;
        };
        let candidates = self.candidates();
        if !candidates.is_empty() {
            match key.code {
                KeyCode::Tab => {
                    let c = &candidates[self.comp_sel.min(candidates.len() - 1)];
                    self.editor.replace_current_word(&c.insert);
                    self.comp_sel = 0;
                    return None;
                }
                KeyCode::Down => {
                    self.comp_sel = (self.comp_sel + 1) % candidates.len();
                    return None;
                }
                KeyCode::Up => {
                    self.comp_sel = (self.comp_sel + candidates.len() - 1) % candidates.len();
                    return None;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Esc {
            self.mode = Mode::Normal;
            if matches!(self.target, Target::Edit(_)) {
                self.editor.clear();
            }
            self.target = Target::New;
            return None;
        }
        // Boş girişte ↑: son kendi mesajını düzenle.
        if key.code == KeyCode::Up && self.editor.is_empty() && self.target == Target::New {
            let last = self
                .shared
                .read()
                .last_own_message(open.channel)
                .map(|m| m.id);
            if let Some(id) = last {
                self.enter_insert(Target::Edit(id));
            }
            return None;
        }
        match self.editor.handle_key(key) {
            EditOutcome::Submit => self.submit(open),
            EditOutcome::Changed => {
                self.comp_sel = 0;
                self.typing_action(open)
            }
            EditOutcome::Ignored => None,
        }
    }

    fn submit(&mut self, open: crate::Open) -> Option<Action> {
        let text = self.editor.text();
        let content = text.trim();
        let target = std::mem::take(&mut self.target);
        if content.is_empty() {
            return None;
        }
        let content = content.to_owned();
        self.editor.clear();
        self.last_typing = None;
        self.scroll = Scroll::default();
        match target {
            Target::Edit(message) => {
                self.mode = Mode::Normal;
                let unchanged = self
                    .shared
                    .read()
                    .find_message(open.channel, message)
                    .is_some_and(|m| m.content == content);
                (!unchanged).then_some(Action::EditMessage {
                    guild: open.guild,
                    channel: open.channel,
                    message,
                    content,
                })
            }
            Target::Reply(id) => Some(Action::SendMessage {
                guild: open.guild,
                channel: open.channel,
                content,
                reply_to: Some(id),
            }),
            Target::New => Some(Action::SendMessage {
                guild: open.guild,
                channel: open.channel,
                content,
                reply_to: None,
            }),
        }
    }

    /// "Yazıyor" bildirimini en fazla `TYPING_EVERY`'de bir ister.
    fn typing_action(&mut self, open: crate::Open) -> Option<Action> {
        if self.editor.is_empty() || matches!(self.target, Target::Edit(_)) {
            return None;
        }
        let now = Instant::now();
        let due = self
            .last_typing
            .is_none_or(|(ch, at)| ch != open.channel || now.duration_since(at) >= TYPING_EVERY);
        if !due {
            return None;
        }
        self.last_typing = Some((open.channel, now));
        Some(Action::Typing {
            guild: open.guild,
            channel: open.channel,
        })
    }

    pub(crate) fn select_key(&mut self, key: KeyEvent) -> Option<Action> {
        let open = self.open?;
        let sel = self.selected?;
        let d_pressed = std::mem::take(&mut self.pending_d);
        match self.keys.lookup(Scope::Select, &key) {
            Some(Act::Back) => self.mode = Mode::Normal,
            Some(Act::MoveDown) => self.move_selection(1),
            Some(Act::MoveUp) => self.move_selection(-1),
            Some(Act::Reply) => self.enter_insert(Target::Reply(sel)),
            Some(Act::Edit) => {
                let own = self.is_own(sel);
                if own {
                    self.enter_insert(Target::Edit(sel));
                } else {
                    self.notice = Some("Sadece kendi mesajını düzenleyebilirsin".into());
                }
            }
            Some(Act::Delete) if d_pressed => {
                if self.is_own(sel) || self.has_perm(Permissions::MANAGE_MESSAGES) {
                    self.mode = Mode::ConfirmDelete(sel);
                } else {
                    self.notice = Some("Bu mesajı silme iznin yok".into());
                }
            }
            Some(Act::Delete) => self.pending_d = true,
            Some(Act::React) => {
                if self.has_perm(Permissions::ADD_REACTIONS) {
                    self.command.clear();
                    self.command.insert_str(":");
                    self.comp_sel = 0;
                    self.mode = Mode::React;
                } else {
                    self.notice = Some("Tepki ekleme iznin yok".into());
                }
            }
            Some(Act::Copy) => {
                let text = self
                    .shared
                    .read()
                    .find_message(open.channel, sel)
                    .map(|m| m.content.clone())?;
                return Some(Action::Copy(text));
            }
            Some(Act::OpenMedia) => return self.open_media(open.channel, sel),
            Some(Act::Retry) if self.shared.read().is_failed(sel) => {
                return Some(Action::Retry {
                    guild: open.guild,
                    channel: open.channel,
                    message: sel,
                });
            }
            _ => {}
        }
        None
    }

    /// Resimli mesajda görüntüleyiciyi açar; yoksa (ya da resimler kapalıysa) ilk bağlantıyı
    /// harici açar.
    fn open_media(&mut self, channel: Snowflake, sel: Snowflake) -> Option<Action> {
        let (items, link) = {
            let st = self.shared.read();
            let msg = st.find_message(channel, sel)?;
            (crate::viewer::items(msg), crate::viewer::external_link(msg))
        };
        if self.images.enabled() && !items.is_empty() {
            self.viewer = Some(crate::viewer::Viewer {
                items,
                index: 0,
                origin: (channel, sel),
            });
            self.mode = Mode::Viewer;
            return None;
        }
        match items.into_iter().next().map(|i| i.original).or(link) {
            Some(url) => Some(Action::OpenExternal(url)),
            None => {
                self.notice = Some("Açılacak resim ya da bağlantı yok".into());
                None
            }
        }
    }

    pub(crate) fn viewer_key(&mut self, key: KeyEvent) -> Option<Action> {
        let viewer = self.viewer.as_mut()?;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.viewer = None;
                self.mode = Mode::Select;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('j') => viewer.step(true),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('k') => viewer.step(false),
            KeyCode::Char('o') => {
                return viewer
                    .current()
                    .map(|i| Action::OpenExternal(i.original.clone()));
            }
            _ => {}
        }
        None
    }

    fn is_own(&self, id: Snowflake) -> bool {
        let Some(open) = self.open else { return false };
        let st = self.shared.read();
        let me = st.me().map(|u| u.id);
        st.find_message(open.channel, id)
            .is_some_and(|m| Some(m.author.id) == me && !m.is_system())
    }

    fn has_perm(&self, p: Permissions) -> bool {
        self.open
            .is_some_and(|o| self.shared.read().my_permissions(o.channel).contains(p))
    }

    /// Seçimi `delta` mesaj kaydırır ve görünür tutar.
    fn move_selection(&mut self, delta: isize) {
        let (Some(open), Some(sel)) = (self.open, self.selected) else {
            return;
        };
        let st = self.shared.read();
        let Some(msgs) = st.messages(open.channel).filter(|m| !m.is_empty()) else {
            return;
        };
        let idx = crate::view::anchor_index(msgs, Some(sel));
        let new = idx.saturating_add_signed(delta).min(msgs.len() - 1);
        let id = msgs[new].id;
        let last = msgs.len() - 1;
        drop(st);
        self.selected = Some(id);
        if let Some((top, bottom)) = self.visible.get()
            && (id < top || id > bottom)
        {
            self.scroll = if new == last {
                Scroll::default()
            } else {
                Scroll {
                    anchor: Some(id),
                    offset: 0,
                }
            };
        }
    }

    pub(crate) fn confirm_key(&mut self, key: KeyEvent, message: Snowflake) -> Option<Action> {
        self.mode = Mode::Select;
        let open = self.open?;
        match key.code {
            KeyCode::Char('y' | 'e' | 'E' | 'Y') | KeyCode::Enter => Some(Action::DeleteMessage {
                guild: open.guild,
                channel: open.channel,
                message,
            }),
            _ => None,
        }
    }

    /// COMMAND (`:`) ve tepki istemi (`+`) için tek satırlık giriş.
    pub(crate) fn command_key(&mut self, key: KeyEvent) -> Option<Action> {
        let react = self.mode == Mode::React;
        if react {
            let candidates = self.candidates();
            match key.code {
                KeyCode::Tab | KeyCode::Enter if !candidates.is_empty() => {
                    let c = &candidates[self.comp_sel.min(candidates.len() - 1)];
                    let insert = c.insert.trim().to_owned();
                    self.mode = Mode::Select;
                    return self.react_action(&insert);
                }
                KeyCode::Down if !candidates.is_empty() => {
                    self.comp_sel = (self.comp_sel + 1) % candidates.len();
                    return None;
                }
                KeyCode::Up if !candidates.is_empty() => {
                    self.comp_sel = (self.comp_sel + candidates.len() - 1) % candidates.len();
                    return None;
                }
                _ => {}
            }
        }
        if key.code == KeyCode::Esc {
            self.mode = if react { Mode::Select } else { Mode::Normal };
            return None;
        }
        // Tek satır: Shift/Alt+Enter de gönderir.
        let outcome = if key.code == KeyCode::Enter {
            EditOutcome::Submit
        } else {
            self.command.handle_key(key)
        };
        match outcome {
            EditOutcome::Submit => {
                let text = self.command.text();
                self.command.clear();
                if react {
                    self.mode = Mode::Select;
                    self.react_action(text.trim())
                } else {
                    self.mode = Mode::Normal;
                    self.run_command(text.trim())
                }
            }
            EditOutcome::Changed => {
                self.comp_sel = 0;
                None
            }
            EditOutcome::Ignored => None,
        }
    }

    /// Tepki istemine yazılan değeri (emoji, `:kısa_ad:` veya `<:ad:id>`) çözer; aynı tepkim varsa kaldırır.
    fn react_action(&mut self, raw: &str) -> Option<Action> {
        let open = self.open?;
        let message = self.selected?;
        let emoji = parse_emoji(raw).or_else(|| {
            self.notice = Some(format!("Tanınmayan emoji: {raw}"));
            None
        })?;
        let mine = self
            .shared
            .read()
            .find_message(open.channel, message)
            .is_some_and(|m| m.reactions.iter().any(|r| r.me && r.emoji == emoji));
        Some(Action::React {
            guild: open.guild,
            channel: open.channel,
            message,
            emoji,
            add: !mine,
        })
    }

    fn run_command(&mut self, cmd: &str) -> Option<Action> {
        let cmd = cmd.strip_prefix(':').unwrap_or(cmd);
        let (name, rest) = cmd.split_once(' ').unwrap_or((cmd, ""));
        match name {
            "q" | "quit" => {
                self.should_quit = true;
                None
            }
            "upload" | "u" => {
                let Some(open) = self.open else {
                    self.notice = Some("Önce bir kanal aç".into());
                    return None;
                };
                if !self.has_perm(Permissions::ATTACH_FILES) {
                    self.notice = Some("Bu kanala dosya ekleme iznin yok".into());
                    return None;
                }
                let (path, content) = split_path(rest.trim());
                if path.is_empty() {
                    self.notice = Some("Kullanım: :upload <yol> [mesaj]".into());
                    return None;
                }
                Some(Action::Upload {
                    guild: open.guild,
                    channel: open.channel,
                    path: expand_home(&path),
                    content,
                    max_bytes: MAX_UPLOAD,
                })
            }
            "settings" => {
                self.mode = Mode::Settings;
                None
            }
            "search" => self.search_command(rest),
            "pins" => self.pins_command(),
            "members" => Some(self.toggle_members()),
            "theme" => {
                let theme = rest.trim();
                if theme.is_empty() {
                    self.notice = Some("Kullanım: :theme <ad> (default: yerleşik)".into());
                    None
                } else {
                    Some(Action::LoadTheme(theme.to_owned()))
                }
            }
            "threads" => match self.thread_parent_target() {
                Some(parent) => self.toggle_threads(parent),
                None => {
                    self.notice = Some(":threads için thread'li bir kanal seç".into());
                    None
                }
            },
            "archived" => match self.thread_parent_target() {
                Some(channel) => {
                    let guild = self.open.and_then(|o| o.guild).or_else(|| {
                        let st = self.shared.read();
                        self.selected_guild(&st)
                    });
                    // Sonuç görünsün diye liste kapalıysa açılır.
                    let collapsed = !self.threads_expanded(&self.shared.read(), channel);
                    if collapsed {
                        self.flip_threads(channel);
                    }
                    self.notice = Some("Arşivli thread'ler yükleniyor…".into());
                    Some(Action::LoadThreads {
                        guild,
                        channel,
                        archived: true,
                    })
                }
                None => {
                    self.notice = Some(":archived için thread'li bir kanal seç".into());
                    None
                }
            },
            "leave" => Some(Action::LeaveVoice),
            "devices" => Some(Action::ListDevices),
            "vol" => self.volume_command(rest.trim()),
            "mute" => Some(Action::ToggleMute),
            "deaf" => Some(Action::ToggleDeaf),
            // Açık DM'de aramaya katıl (aramayı başlatmak karşı tarafı çaldırmaz; doğrulanmadı).
            "call" => match self.open {
                Some(o) if o.guild.is_none() => Some(Action::JoinVoice {
                    guild: None,
                    channel: o.channel,
                }),
                _ => {
                    self.notice = Some(":call sadece DM'de".into());
                    None
                }
            },
            "" => None,
            other => {
                self.notice = Some(format!("Bilinmeyen komut: :{other}"));
                None
            }
        }
    }

    /// `:vol <ad> <0-200>`: açık ses kanalındaki bir kişinin ses seviyesi (yüzde).
    fn volume_command(&mut self, rest: &str) -> Option<Action> {
        let usage = || Some("Kullanım: :vol <ad> <0-200>".to_owned());
        let Some((name, pct)) = rest.rsplit_once(' ') else {
            self.notice = usage();
            return None;
        };
        let Ok(pct) = pct.trim_end_matches('%').parse::<u16>() else {
            self.notice = usage();
            return None;
        };
        let name = name.trim().trim_start_matches('@').to_lowercase();
        let st = self.shared.read();
        let Some(call) = st.voice_call() else {
            drop(st);
            self.notice = Some("Ses kanalında değilsin".into());
            return None;
        };
        let user = st.voice_members(call.channel).into_iter().find_map(|vs| {
            let u = st.user(vs.user_id)?;
            let shown = st.author_name(call.guild, u).to_lowercase();
            (shown == name || u.username.to_lowercase() == name).then_some(vs.user_id)
        });
        drop(st);
        match user {
            Some(user) => Some(Action::SetVolume {
                user,
                volume: f32::from(pct.min(200)) / 100.0,
            }),
            None => {
                self.notice = Some(format!("Ses kanalında \"{name}\" yok"));
                None
            }
        }
    }

    /// Geçerli kelimeye göre tamamlama adayları (INSERT'te `@`, `#`, `:`; tepki isteminde `:`).
    pub(crate) fn candidates(&self) -> Vec<Candidate> {
        self.candidates_in(&self.shared.read())
    }

    /// [`App::candidates`]; tutulan okuma korumasıyla (çizim yolu).
    pub(crate) fn candidates_in(&self, st: &State) -> Vec<Candidate> {
        let (word, _) = match self.mode {
            Mode::Insert => self.editor.current_word(),
            Mode::React => self.command.current_word(),
            _ => return Vec::new(),
        };
        let Some(open) = self.open else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(prefix) = word.strip_prefix('@').filter(|p| !p.is_empty())
            && self.mode == Mode::Insert
        {
            for (id, name) in st.mention_candidates(open.channel, prefix, MAX_CANDIDATES) {
                out.push(Candidate {
                    label: format!("@{name}"),
                    insert: format!("<@{id}> "),
                });
            }
        } else if let Some(prefix) = word.strip_prefix('#').filter(|p| !p.is_empty())
            && self.mode == Mode::Insert
            && let Some(gid) = open.guild
        {
            for (id, name) in st.channel_candidates(gid, prefix, MAX_CANDIDATES) {
                out.push(Candidate {
                    label: format!("#{name}"),
                    insert: format!("<#{id}> "),
                });
            }
        } else if let Some(prefix) = word.strip_prefix(':').filter(|p| p.chars().count() >= 2) {
            let prefix = prefix.trim_end_matches(':').to_lowercase();
            if let Some(gid) = open.guild {
                for e in st
                    .guild_emojis(gid)
                    .iter()
                    .filter(|e| e.available && e.name.to_lowercase().starts_with(&prefix))
                    .take(MAX_CANDIDATES)
                {
                    let a = if e.animated { "a" } else { "" };
                    out.push(Candidate {
                        label: format!(":{}: (sunucu)", e.name),
                        insert: format!("<{a}:{}:{}> ", e.name, e.id),
                    });
                }
            }
            // Önce tam eşleşme, sonra kısa adlar (":fire" → 🔥, 🧑‍🚒 değil).
            let mut found: Vec<(&str, &str)> = emojis::iter()
                .filter_map(|e| {
                    e.shortcodes()
                        .find(|c| c.starts_with(&prefix))
                        .map(|c| (c, e.as_str()))
                })
                .collect();
            found.sort_by_key(|(code, _)| (*code != prefix, code.len()));
            for (code, emoji) in found {
                if out.len() >= MAX_CANDIDATES {
                    break;
                }
                out.push(Candidate {
                    label: format!("{emoji} :{code}:"),
                    insert: format!("{emoji} "),
                });
            }
        }
        out.truncate(MAX_CANDIDATES);
        out
    }
}

/// Hızlı geçiş adayı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwitchItem {
    pub guild: Option<Snowflake>,
    pub channel: Snowflake,
    pub label: String,
}

impl App {
    /// Tüm okunabilir kanallar: DM'ler ve sunucu kanalları ("#kanal · Sunucu").
    fn switch_items(&self) -> Vec<SwitchItem> {
        let st = self.shared.read();
        let mut out: Vec<SwitchItem> = st
            .dm_channels()
            .iter()
            .map(|c| SwitchItem {
                guild: None,
                channel: *c,
                label: format!("@{}", st.channel_title(*c)),
            })
            .collect();
        for g in st.guilds() {
            for item in st.guild_tree(g.id) {
                if item.kind == digcord_store::TreeKind::Channel
                    && matches!(
                        item.channel_kind,
                        digcord_model::channel_kind::TEXT
                            | digcord_model::channel_kind::ANNOUNCEMENT
                    )
                {
                    out.push(SwitchItem {
                        guild: Some(g.id),
                        channel: item.id,
                        label: format!("#{} · {}", item.name, g.name),
                    });
                }
            }
        }
        out
    }

    /// Sorguya göre sıralı en fazla 12 aday.
    pub(crate) fn switcher_matches(&self) -> Vec<SwitchItem> {
        let items = self.switch_items();
        let query = self.command.text();
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        crate::fuzzy::rank(query.trim(), &labels, 12)
            .into_iter()
            .filter_map(|i| items.get(i).cloned())
            .collect()
    }

    pub(crate) fn switcher_key(&mut self, key: KeyEvent) -> Option<Action> {
        let matches = self.switcher_matches();
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                None
            }
            KeyCode::Enter => {
                let item = matches.get(self.comp_sel.min(matches.len().saturating_sub(1)))?;
                Some(self.open_channel(item.guild, item.channel))
            }
            KeyCode::Down | KeyCode::Tab if !matches.is_empty() => {
                self.comp_sel = (self.comp_sel + 1) % matches.len();
                None
            }
            KeyCode::Up | KeyCode::BackTab if !matches.is_empty() => {
                self.comp_sel = (self.comp_sel + matches.len() - 1) % matches.len();
                None
            }
            _ => {
                if self.command.handle_key(key) == EditOutcome::Changed {
                    self.comp_sel = 0;
                }
                None
            }
        }
    }
}

/// `👍`, `:thumbsup:`, `<:ad:id>`, `<a:ad:id>`, `ad:id` → tepki emojisi.
pub(crate) fn parse_emoji(raw: &str) -> Option<ReactionEmoji> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let custom = raw
        .strip_prefix("<a:")
        .or_else(|| raw.strip_prefix("<:"))
        .and_then(|r| r.strip_suffix('>'))
        .or_else(|| {
            raw.contains(':')
                .then_some(raw)
                .filter(|r| !r.starts_with(':'))
        });
    if let Some(body) = custom
        && let Some((name, id)) = body.rsplit_once(':')
        && let Ok(id) = id.parse::<u64>()
    {
        return Some(ReactionEmoji {
            id: Some(Snowflake(id)),
            name: Some(name.to_owned()),
            animated: raw.starts_with("<a:"),
        });
    }
    if let Some(code) = raw.strip_prefix(':').map(|c| c.trim_end_matches(':')) {
        return emojis::get_by_shortcode(code).map(|e| ReactionEmoji::unicode(e.as_str()));
    }
    emojis::get(raw).map(|e| ReactionEmoji::unicode(e.as_str()))
}

/// `"~/a b.png" metin` veya `~/a.png metin` → (yol, metin).
fn split_path(rest: &str) -> (String, String) {
    if let Some(r) = rest.strip_prefix('"')
        && let Some((path, after)) = r.split_once('"')
    {
        return (path.to_owned(), after.trim().to_owned());
    }
    match rest.split_once(' ') {
        Some((p, m)) => (p.to_owned(), m.trim().to_owned()),
        None => (rest.to_owned(), String::new()),
    }
}

fn expand_home(path: &str) -> std::path::PathBuf {
    match path.strip_prefix("~/") {
        Some(rest) => dirs_home().join(rest),
        None => std::path::PathBuf::from(path),
    }
}

fn dirs_home() -> std::path::PathBuf {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default()
}

/// Modu durum çubuğunda gösterilecek etiket.
pub(crate) fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => " [NORMAL] ",
        Mode::Insert => " [YAZ] ",
        Mode::Select => " [SEÇ] ",
        Mode::Command => " [KOMUT] ",
        Mode::React => " [TEPKİ] ",
        Mode::ConfirmDelete(_) => " [SİL?] ",
        Mode::Switcher => " [GEÇİŞ] ",
        Mode::Viewer => " [RESİM] ",
        Mode::Settings => " [AYAR] ",
        Mode::Search => " [ARAMA] ",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_emoji_forms() {
        assert_eq!(parse_emoji("👍"), Some(ReactionEmoji::unicode("👍")));
        assert_eq!(
            parse_emoji(":thumbsup:"),
            Some(ReactionEmoji::unicode("👍"))
        );
        assert_eq!(parse_emoji(":+1"), Some(ReactionEmoji::unicode("👍")));
        let c = parse_emoji("<a:kedi:77>").expect("özel");
        assert_eq!(
            (c.id, c.name.as_deref(), c.animated),
            (Some(Snowflake(77)), Some("kedi"), true)
        );
        assert_eq!(
            parse_emoji("kedi:77").and_then(|e| e.id),
            Some(Snowflake(77))
        );
        assert_eq!(parse_emoji(":yok_boyle_bir_emoji:"), None);
        assert_eq!(parse_emoji(""), None);
    }

    #[test]
    fn splits_upload_arguments() {
        assert_eq!(
            split_path("~/a.png selam"),
            ("~/a.png".into(), "selam".into())
        );
        assert_eq!(
            split_path("\"/tmp/a b.png\" metin burada"),
            ("/tmp/a b.png".into(), "metin burada".into())
        );
        assert_eq!(split_path("/x"), ("/x".into(), String::new()));
    }
}
