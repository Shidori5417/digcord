//! Geçici bildirim kutuları (toast): mesaj alanının sağ üstünde üst üste dizilir, birkaç saniye
//! sonra kendiliğinden kaybolur. Süre kontrolü açık `Instant` ile yapılır (testlenebilir).

use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::members::fit;
use crate::theme::Theme;

/// Bir bildirimin ömrü.
pub(crate) const LIFETIME: Duration = Duration::from_secs(4);
/// Aynı anda en çok bu kadar bildirim görünür (en yenisi üstte).
pub(crate) const MAX_VISIBLE: usize = 3;
/// Kutunun en büyük genişliği (kenarlıklar dahil).
const MAX_WIDTH: u16 = 44;
/// Bu genişliğin altındaki alana bildirim çizilmez.
const MIN_AREA_WIDTH: u16 = 20;
/// Gövde en çok bu kadar satır.
const BODY_ROWS: usize = 2;

/// Bildirimin türü (renk ve çerçeveyi belirler).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    /// Yeni DM ya da seni anan mesaj.
    Message,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    pub title: String,
    pub body: String,
    pub kind: ToastKind,
    expires: Instant,
}

/// Görünür bildirimler; en yenisi sonda.
#[derive(Debug, Default)]
pub(crate) struct Toasts {
    items: Vec<Toast>,
}

impl Toasts {
    pub(crate) fn push(&mut self, title: String, body: String, kind: ToastKind, now: Instant) {
        self.items.push(Toast {
            title,
            body,
            kind,
            expires: now + LIFETIME,
        });
        if self.items.len() > MAX_VISIBLE {
            let extra = self.items.len() - MAX_VISIBLE;
            self.items.drain(..extra);
        }
    }

    /// Süresi dolanları atar; bir şey değiştiyse `true`.
    pub(crate) fn expire(&mut self, now: Instant) -> bool {
        let before = self.items.len();
        self.items.retain(|t| t.expires > now);
        self.items.len() != before
    }

    /// En yakın sona erme anı (boşsa `None`); çalışma döngüsü o ana kadar uyanmaz.
    pub(crate) fn next_expiry(&self) -> Option<Instant> {
        self.items.iter().map(|t| t.expires).min()
    }

    /// Yukarıdan aşağıya çizim sırası: en yeni önce.
    pub(crate) fn newest_first(&self) -> impl Iterator<Item = &Toast> {
        self.items.iter().rev()
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }
}

/// Gövdeyi `width` sütuna, en çok [`BODY_ROWS`] satıra böler; sığmayan sondan `…` ile kesilir.
fn wrap_body(body: &str, width: usize) -> Vec<String> {
    use unicode_width::UnicodeWidthChar;
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut used = 0;
    let mut truncated = false;
    for c in body.chars().map(|c| if c.is_control() { ' ' } else { c }) {
        let w = c.width().unwrap_or(0);
        if used + w > width {
            if rows.len() + 1 == BODY_ROWS {
                truncated = true;
                break;
            }
            rows.push(std::mem::take(&mut cur));
            used = 0;
        }
        cur.push(c);
        used += w;
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    if truncated && let Some(last) = rows.last_mut() {
        last.push('…');
        *last = fit(last, width);
    }
    rows
}

/// Bildirimleri `area`nın sağ üstüne çizer (`area`: kenarlıksız iç alan).
pub(crate) fn draw(toasts: &Toasts, theme: &Theme, frame: &mut Frame, area: Rect) {
    if area.width < MIN_AREA_WIDTH {
        return;
    }
    let width = area.width.min(MAX_WIDTH);
    let x = area.right() - width;
    let mut y = area.y;
    for t in toasts.newest_first() {
        let rows = wrap_body(&t.body, usize::from(width.saturating_sub(2)));
        let height = 2 + 1 + u16::try_from(rows.len()).unwrap_or(0);
        if y + height > area.bottom() {
            break;
        }
        let rect = Rect::new(x, y, width, height);
        let accent = match t.kind {
            ToastKind::Error => theme.error(),
            ToastKind::Warning => theme.warning(),
            ToastKind::Message | ToastKind::Info => theme.toast_info(),
        };
        frame.render_widget(Clear, rect);
        let block = Block::bordered()
            .border_style(accent)
            .style(theme.toast_bg());
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        let mut lines = vec![Line::styled(
            fit(&t.title, usize::from(inner.width)),
            accent.add_modifier(Modifier::BOLD),
        )];
        lines.extend(rows.into_iter().map(Line::from));
        frame.render_widget(Paragraph::new(lines), inner);
        y += height;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacks_newest_last_and_caps_at_three() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        for i in 0..5 {
            ts.push(format!("t{i}"), String::new(), ToastKind::Info, t0);
        }
        assert_eq!(ts.len(), MAX_VISIBLE);
        let order: Vec<&str> = ts.newest_first().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["t4", "t3", "t2"]);
    }

    #[test]
    fn expires_by_explicit_time() {
        let t0 = Instant::now();
        let mut ts = Toasts::default();
        ts.push("a".into(), String::new(), ToastKind::Info, t0);
        ts.push(
            "b".into(),
            String::new(),
            ToastKind::Error,
            t0 + Duration::from_secs(2),
        );
        assert_eq!(ts.next_expiry(), Some(t0 + LIFETIME));
        assert!(!ts.expire(t0 + Duration::from_secs(3)));
        assert_eq!(ts.len(), 2);
        // Sınırda (expires == now) kaybolur.
        assert!(ts.expire(t0 + LIFETIME));
        assert_eq!(ts.len(), 1);
        assert_eq!(
            ts.next_expiry(),
            Some(t0 + Duration::from_secs(2) + LIFETIME)
        );
        assert!(ts.expire(t0 + Duration::from_secs(60)));
        assert_eq!(ts.len(), 0);
        assert_eq!(ts.next_expiry(), None);
        assert!(!ts.expire(t0 + Duration::from_secs(61)));
    }

    #[test]
    fn body_wraps_and_truncates() {
        assert_eq!(wrap_body("kısa", 10), ["kısa"]);
        assert_eq!(wrap_body("", 10), [""]);
        assert_eq!(wrap_body("abcdefghijkl", 10), ["abcdefghij", "kl"]);
        let rows = wrap_body(&"x".repeat(50), 10);
        assert_eq!(rows.len(), BODY_ROWS);
        assert!(rows[1].ends_with('…'));
        assert_eq!(rows[1].chars().count(), 10);
        assert_eq!(wrap_body("a\nb", 10), ["a b"]);
    }
}
