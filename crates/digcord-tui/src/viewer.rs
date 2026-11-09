//! Tam ekran resim görüntüleyici (SELECT'te `o`).

use digcord_model::{Message, Snowflake};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::markdown::{self, Span as Md};
use crate::media::{Images, SlotState};

/// Mesajdaki gösterilebilir bir resim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    /// Vekil adres (boyut parametresi eklenir).
    pub src: String,
    pub width: u32,
    pub height: u32,
    /// Harici açılacak adres.
    pub original: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Viewer {
    pub items: Vec<Item>,
    pub index: usize,
    pub origin: (Snowflake, Snowflake),
}

impl Viewer {
    pub(crate) fn current(&self) -> Option<&Item> {
        self.items.get(self.index)
    }

    pub(crate) fn step(&mut self, forward: bool) {
        let n = self.items.len();
        if n > 0 {
            self.index = if forward {
                (self.index + 1) % n
            } else {
                (self.index + n - 1) % n
            };
        }
    }
}

/// Mesajın resim ekleri ve embed resimleri.
pub(crate) fn items(msg: &Message) -> Vec<Item> {
    let mut out = Vec::new();
    for a in &msg.attachments {
        if let (true, Some(w), Some(h)) = (a.is_image(), a.width, a.height) {
            let src = if a.proxy_url.is_empty() {
                &a.url
            } else {
                &a.proxy_url
            };
            out.push(Item {
                src: src.clone(),
                width: w,
                height: h,
                original: a.url.clone(),
                name: a.filename.clone(),
            });
        }
    }
    for e in &msg.embeds {
        let media = [&e.image, &e.thumbnail]
            .into_iter()
            .flatten()
            .find_map(|m| Some((m.proxy_url.clone()?, m.width?, m.height?, m.url.clone())));
        if let Some((src, w, h, url)) = media {
            let original = e.url.clone().or(url).unwrap_or_else(|| src.clone());
            out.push(Item {
                name: e.title.clone().unwrap_or_else(|| "embed".into()),
                src,
                width: w,
                height: h,
                original,
            });
        }
    }
    out
}

/// Resim yoksa `o` ile açılacak adres: içerikteki ilk bağlantı, yoksa ilk ek.
pub(crate) fn external_link(msg: &Message) -> Option<String> {
    markdown::parse(&msg.content)
        .iter()
        .flat_map(|b| b.spans.iter())
        .find_map(|s| match s {
            Md::Link(url) => Some(url.clone()),
            _ => None,
        })
        .or_else(|| msg.attachments.first().map(|a| a.url.clone()))
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
}

pub(crate) fn draw(viewer: &Viewer, images: &Images, frame: &mut Frame) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let Some(item) = viewer.current() else { return };
    let title = format!(
        " {} ({}/{}) — ←/→ gez · o: harici aç · Esc: kapat ",
        item.name,
        viewer.index + 1,
        viewer.items.len()
    );
    let block = Block::bordered().title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(fit) = images.fit_area(item.width, item.height, inner) else {
        return;
    };
    // Ortala.
    let x = inner.x + (inner.width - fit.cols) / 2;
    let y = inner.y + (inner.height - fit.rows) / 2;
    let target = Rect::new(x, y, fit.cols, fit.rows);
    let url = crate::render::sized_url(&item.src, fit.px_w, fit.px_h);
    let note = match images.draw(frame, target, &url, Some(viewer.origin)) {
        SlotState::Drawn => return,
        SlotState::Loading => "yükleniyor…",
        SlotState::Failed => "resim yüklenemedi (o: harici aç)",
    };
    let row = Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1);
    frame.render_widget(Paragraph::new(Line::from(note).dim().centered()), row);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(v: serde_json::Value) -> Message {
        serde_json::from_value(v).expect("mesaj")
    }

    #[test]
    fn collects_images_and_links() {
        let m = msg(serde_json::json!({
            "id": "1", "channel_id": "2", "author": {"id": "4", "username": "a"},
            "content": "bak https://x.example/sayfa",
            "attachments": [
                {"id": "9", "filename": "a.png", "width": 4, "height": 2,
                 "content_type": "image/png", "url": "https://cdn.discordapp.com/a.png",
                 "proxy_url": "https://media.discordapp.net/a.png"},
                {"id": "10", "filename": "b.zip", "url": "https://cdn.discordapp.com/b.zip"}
            ],
            "embeds": [{"type": "image", "url": "https://x.example/c.png",
                "thumbnail": {"url": "https://x.example/c.png",
                "proxy_url": "https://images-ext-1.discordapp.net/c.png", "width": 8, "height": 8}}]
        }));
        let items = items(&m);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].src, "https://media.discordapp.net/a.png");
        assert_eq!(items[0].original, "https://cdn.discordapp.com/a.png");
        assert_eq!(items[1].original, "https://x.example/c.png");
        assert_eq!(
            external_link(&m).as_deref(),
            Some("https://x.example/sayfa")
        );
    }

    #[test]
    fn link_falls_back_to_attachment_and_rejects_other_schemes() {
        let m = msg(serde_json::json!({
            "id": "1", "channel_id": "2", "author": {"id": "4", "username": "a"},
            "content": "dosya",
            "attachments": [{"id": "10", "filename": "b.zip", "url": "https://cdn.discordapp.com/b.zip"}]
        }));
        assert_eq!(
            external_link(&m).as_deref(),
            Some("https://cdn.discordapp.com/b.zip")
        );
        let m = msg(serde_json::json!({
            "id": "1", "channel_id": "2", "author": {"id": "4", "username": "a"},
            "content": "yok"
        }));
        assert_eq!(external_link(&m), None);
    }

    #[test]
    fn step_wraps() {
        let item = Item {
            src: String::new(),
            width: 1,
            height: 1,
            original: String::new(),
            name: String::new(),
        };
        let mut v = Viewer {
            items: vec![item.clone(), item],
            index: 0,
            origin: (Snowflake(1), Snowflake(2)),
        };
        v.step(false);
        assert_eq!(v.index, 1);
        v.step(true);
        assert_eq!(v.index, 0);
    }
}
