//! `:search` ve `:pins`: tam ekran sonuç listesi ve bir sonuca atlama.
//!
//! Ağ yok: sorgular [`Action`] olarak denetleyiciye gider, sonuçlar [`UiEvent`] ile döner
//! (`SearchResults`/`SearchFailed`). Her kullanıcı eylemi (komut, `n`/`p`) tek istek doğurur;
//! eski bir isteğin geç gelen yanıtı `seq` ile ayıklanır.

use digcord_model::{Message, RawSearch, SearchQuery, Snowflake};
use digcord_store::{State, TreeKind};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthChar;

use crate::compose::Mode;
use crate::{Action, App, Focus, Scroll, view};

/// Sunucunun sayfa başına döndürdüğü sonuç sayısı (`offset` adımı; doğrulanmadı).
pub(crate) const PAGE: u32 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Search,
    Pins,
}

/// Açık sonuç ekranının durumu.
#[derive(Debug)]
pub(crate) struct SearchView {
    pub(crate) kind: Kind,
    /// Aramanın sunucusu (`None`: DM kanalı araması). Sabitlenenlerde açık kanalın sunucusu.
    pub(crate) guild: Option<Snowflake>,
    /// Komutun verildiği kanal (isteğin `Referer`'i ve DM araması kapsamı).
    pub(crate) channel: Snowflake,
    pub(crate) query: SearchQuery,
    /// Başlıkta gösterilen sorgu metni.
    pub(crate) text: String,
    pub(crate) seq: u64,
    pub(crate) offset: u32,
    pub(crate) total: u64,
    pub(crate) hits: Vec<Message>,
    pub(crate) sel: usize,
    pub(crate) loading: bool,
    pub(crate) error: Option<String>,
}

impl SearchView {
    pub(crate) fn set_results(&mut self, total: u64, hits: Vec<Message>) {
        self.total = total;
        self.hits = hits;
        self.sel = 0;
        self.loading = false;
        self.error = None;
    }

    pub(crate) fn set_error(&mut self, error: String) {
        self.loading = false;
        self.error = Some(error);
    }

    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }
}

impl App {
    /// `:search <sorgu>`: süzgeçleri kimliğe çözer, ekranı "aranıyor" ile açar, isteği yollar.
    pub(crate) fn search_command(&mut self, rest: &str) -> Option<Action> {
        let Some(open) = self.open else {
            self.notice = Some("Önce bir kanal aç".into());
            return None;
        };
        let raw = digcord_model::parse_search(rest);
        if !raw.bad_has.is_empty() {
            self.notice = Some(format!(
                "Bilinmeyen has: değeri: {} (link, file, image, embed)",
                raw.bad_has.join(", ")
            ));
            return None;
        }
        let query = match self.resolve_query(&raw, open.guild) {
            Ok(q) => q,
            Err(e) => {
                self.notice = Some(e);
                return None;
            }
        };
        if query.is_empty() {
            self.notice = Some("Kullanım: :search <sözcükler> [from:<kimlik|@ad>] [in:#kanal] [has:link|file|image|embed]".into());
            return None;
        }
        self.search_seq += 1;
        let view = SearchView {
            kind: Kind::Search,
            guild: open.guild,
            channel: open.channel,
            query: query.clone(),
            text: rest.trim().to_owned(),
            seq: self.search_seq,
            offset: 0,
            total: 0,
            hits: Vec::new(),
            sel: 0,
            loading: true,
            error: None,
        };
        let action = Action::Search {
            seq: view.seq,
            guild: open.guild,
            channel: open.channel,
            query,
            offset: 0,
        };
        self.search = Some(view);
        self.mode = Mode::Search;
        Some(action)
    }

    /// `:pins`: açık kanalın sabitlenmiş mesajları.
    pub(crate) fn pins_command(&mut self) -> Option<Action> {
        let Some(open) = self.open else {
            self.notice = Some("Önce bir kanal aç".into());
            return None;
        };
        self.search_seq += 1;
        self.search = Some(SearchView {
            kind: Kind::Pins,
            guild: open.guild,
            channel: open.channel,
            query: SearchQuery::default(),
            text: String::new(),
            seq: self.search_seq,
            offset: 0,
            total: 0,
            hits: Vec::new(),
            sel: 0,
            loading: true,
            error: None,
        });
        self.mode = Mode::Search;
        Some(Action::LoadPins {
            seq: self.search_seq,
            guild: open.guild,
            channel: open.channel,
        })
    }

    /// `from:` / `in:` değerlerini kimliklere çevirir (hata metni kullanıcıya gösterilir).
    fn resolve_query(
        &self,
        raw: &RawSearch,
        guild: Option<Snowflake>,
    ) -> Result<SearchQuery, String> {
        let st = self.shared.read();
        let author = match raw.from.as_deref() {
            None => None,
            Some(v) => Some(
                digcord_model::parse_user_ref(v)
                    .or_else(|| st.find_user_by_name(v).map(|u| u.id))
                    .ok_or_else(|| {
                        format!("from: kullanıcı bulunamadı: {v} (kimlik ya da bilinen ad)")
                    })?,
            ),
        };
        let channel = match (raw.channel.as_deref(), guild) {
            (None, _) => None,
            (Some(_), None) => return Err("in: sadece sunucu aramasında kullanılır".into()),
            (Some(v), Some(g)) => Some(
                digcord_model::parse_channel_ref(v)
                    .or_else(|| channel_by_name(&st, g, v))
                    .ok_or_else(|| format!("in: kanal bulunamadı: {v}"))?,
            ),
        };
        Ok(SearchQuery {
            content: raw.content.clone(),
            author,
            channel,
            has: raw.has.clone(),
        })
    }

    pub(crate) fn search_key(&mut self, key: KeyEvent) -> Option<Action> {
        let Some(view) = self.search.as_mut() else {
            self.mode = Mode::Normal;
            return None;
        };
        let last = view.hits.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.search = None;
                self.mode = Mode::Normal;
            }
            KeyCode::Char('j') | KeyCode::Down => view.sel = (view.sel + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => view.sel = view.sel.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => view.sel = 0,
            KeyCode::Char('G') | KeyCode::End => view.sel = last,
            KeyCode::Char('n') | KeyCode::PageDown => return self.search_page(true),
            KeyCode::Char('p') | KeyCode::PageUp => return self.search_page(false),
            KeyCode::Enter => {
                let msg = view.hits.get(view.sel)?.clone();
                let fallback = view.guild;
                self.search = None;
                self.mode = Mode::Normal;
                return self.jump_to(&msg, fallback);
            }
            _ => {}
        }
        None
    }

    /// `n`/`p`: sonraki/önceki sayfa (yalnızca aramada); yükleme sürerken yok sayılır.
    fn search_page(&mut self, forward: bool) -> Option<Action> {
        let view = self.search.as_mut()?;
        if view.kind != Kind::Search || view.loading {
            return None;
        }
        let offset = if forward {
            let next = view.offset + PAGE;
            if u64::from(next) >= view.total {
                self.notice = Some("Son sayfa".into());
                return None;
            }
            next
        } else if view.offset == 0 {
            self.notice = Some("İlk sayfa".into());
            return None;
        } else {
            view.offset.saturating_sub(PAGE)
        };
        self.search_seq += 1;
        view.seq = self.search_seq;
        view.offset = offset;
        view.loading = true;
        view.error = None;
        Some(Action::Search {
            seq: view.seq,
            guild: view.guild,
            channel: view.channel,
            query: view.query.clone(),
            offset,
        })
    }

    /// Mesajın kanalını açıp mesaja kaydırır ve seçer. Mesaj yüklü değilse
    /// [`Action::JumpTo`] ile çevresi (`around`) istenir; gelince [`App::apply_jump`] kaydırır.
    pub(crate) fn jump_to(
        &mut self,
        msg: &Message,
        fallback_guild: Option<Snowflake>,
    ) -> Option<Action> {
        let channel = msg.channel_id;
        let guild = {
            let st = self.shared.read();
            st.channel(channel)
                .map(|c| c.guild_id)
                .unwrap_or(msg.guild_id.or(fallback_guild))
        };
        let same = self.open.is_some_and(|o| o.channel == channel);
        let previous = self.open.map(|o| o.channel).filter(|c| *c != channel);
        let open_action = if same {
            None
        } else {
            Some(self.open_channel(guild, channel))
        };
        self.pending_jump = Some((channel, msg.id));
        self.apply_jump();
        if self.pending_jump.is_none() {
            // Mesaj zaten yüklüydü: kanal açıldıysa sadece açma eylemi gerekir.
            return open_action;
        }
        self.notice = Some("Mesaj yükleniyor…".into());
        Some(Action::JumpTo {
            guild,
            channel,
            message: msg.id,
            previous,
        })
    }

    /// Bekleyen atlama hedefi store'da varsa görünümü ona hizalar ve SEÇ moduna geçer.
    pub(crate) fn apply_jump(&mut self) {
        let Some((channel, id)) = self.pending_jump else {
            return;
        };
        let Some(open) = self.open.filter(|o| o.channel == channel) else {
            self.pending_jump = None;
            return;
        };
        let (width, height) = self.msg_area.get();
        let st = self.shared.read();
        let Some(msgs) = st.messages(channel) else {
            return;
        };
        let Ok(idx) = msgs.binary_search_by_key(&id, |m| m.id) else {
            return;
        };
        // Hedef ekranın ortasına gelsin: altına yarım ekran kadar mesaj sığdır.
        let scroll = if width == 0 || height == 0 {
            Scroll {
                anchor: Some(id),
                offset: 0,
            }
        } else {
            let res = self.res(&st, open);
            let (mut last, mut used) = (idx, 0);
            while last + 1 < msgs.len() && used < usize::from(height / 2) {
                last += 1;
                used += view::height(msgs, last, &res, width);
            }
            if last + 1 >= msgs.len() {
                Scroll::default()
            } else {
                Scroll {
                    anchor: Some(msgs[last].id),
                    offset: 0,
                }
            }
        };
        let detached = st.is_detached(channel);
        drop(st);
        self.scroll = scroll;
        self.selected = Some(id);
        self.mode = Mode::Select;
        self.focus = Focus::Messages;
        self.pending_d = false;
        self.pending_jump = None;
        if detached {
            self.notice = Some("Eski mesajlardasın; güncele dönmek için G".into());
        }
    }
}

fn channel_by_name(st: &State, guild: Snowflake, name: &str) -> Option<Snowflake> {
    let name = name.trim_start_matches('#').to_lowercase();
    st.guild_tree(guild)
        .iter()
        .find(|i| i.kind == TreeKind::Channel && i.name.to_lowercase() == name)
        .map(|i| i.id)
}

/// Genişliğe sığdırır (sığmazsa `…`); geniş karakterler hesaba katılır.
fn fit(s: &str, width: usize) -> String {
    let total: usize = s.chars().map(|c| c.width().unwrap_or(0)).sum();
    if total <= width {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

/// Tek satırlık özet: `#kanal  Yazar  gg.aa.yyyy ss:dd  içerik`.
fn hit_line(st: &State, tz: &jiff::tz::TimeZone, guild: Option<Snowflake>, m: &Message) -> String {
    let place = {
        let t = st.channel_title(m.channel_id);
        let in_guild = st
            .channel(m.channel_id)
            .map_or(m.guild_id.or(guild).is_some(), |c| c.guild_id.is_some());
        if in_guild {
            format!("#{t}")
        } else {
            format!("@{t}")
        }
    };
    let author = st.author_name(m.guild_id.or(guild), &m.author);
    let time = i64::try_from(m.timestamp_ms())
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map(|t| {
            t.to_zoned(tz.clone())
                .strftime("%d.%m.%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let body: String = if m.content.trim().is_empty() {
        "(ek veya embed)".to_owned()
    } else {
        m.content.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    format!("{place}  {author}  {time}  {body}")
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let Some(v) = &app.search else { return };
    let t = &app.theme;
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, t.base());
    let title = match v.kind {
        Kind::Search => format!(
            " Arama: {} — {} sonuç{} ",
            v.text,
            v.total,
            if v.hits.is_empty() {
                String::new()
            } else {
                let from = u64::from(v.offset) + 1;
                format!(" ({from}–{})", u64::from(v.offset) + v.hits.len() as u64)
            }
        ),
        Kind::Pins => {
            let st = app.shared.read();
            format!(
                " Sabitlenenler ({}) — {} ",
                st.channel_title(v.channel),
                v.hits.len()
            )
        }
    };
    let block = Block::bordered()
        .title(title)
        .border_style(t.border_style(true));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    let width = usize::from(list.width);
    let h = usize::from(list.height).max(1);
    let lines: Vec<Line> = if let Some(e) = &v.error {
        vec![Line::styled(e.clone(), t.warning())]
    } else if v.loading {
        vec![Line::styled("Yükleniyor…", t.dim())]
    } else if v.hits.is_empty() {
        vec![Line::styled("Sonuç yok", t.dim())]
    } else {
        let st = app.shared.read();
        let offset = v.sel.saturating_sub(h - 1);
        app.hits.overlay_list.set(list);
        app.hits.overlay_offset.set(offset);
        v.hits
            .iter()
            .enumerate()
            .skip(offset)
            .take(h)
            .map(|(i, m)| {
                let cursor = if i == v.sel { "> " } else { "  " };
                let style = if i == v.sel {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                let text = hit_line(&st, &app.tz, v.guild, m);
                Line::from(Span::styled(
                    format!("{cursor}{}", fit(&text, width.saturating_sub(2))),
                    style,
                ))
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), list);
    let keys = match v.kind {
        Kind::Search => "j/k gez · Enter mesaja git · n/p sonraki/önceki sayfa · Esc kapat",
        Kind::Pins => "j/k gez · Enter mesaja git · Esc kapat",
    };
    frame.render_widget(Paragraph::new(Line::styled(keys, t.dim())), help);
}
