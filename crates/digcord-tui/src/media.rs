//! Mesajlardaki resimler: terminal grafik yeteneği, hazır resimlerin LRU'su ve indirme istekleri.
//!
//! UI thread'i sadece hazır (kodlanmış) resmi yerine koyar. İndirme, çözme, ölçekleme ve
//! terminal protokolüne kodlama [`ImageJob::encode`] ile denetleyicinin arka plan işinde olur.

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt;

use digcord_media::Lru;
use digcord_model::Snowflake;
use ratatui::Frame;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};

use crate::UiEvent;
use crate::render::ImageFit;

/// Bellekte tutulan hazır resim sayısı (bir ekran dolusunun birkaç katı).
const CACHE_ENTRIES: usize = 96;
/// `thumbnail` kipinde önizlemenin en fazla genişliği (sütun).
const THUMBNAIL_COLS: u16 = 48;

/// `ui.images` ayarı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageMode {
    /// Resim yok; sadece `[ek: ...]` satırları.
    Off,
    /// Küçük önizleme (en fazla `THUMBNAIL_COLS` sütun).
    #[default]
    Thumbnail,
    /// Mesaj alanının tüm genişliği.
    Full,
}

/// TUI ayarları (config.toml `[ui]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub images: ImageMode,
    /// Bir resmin en fazla satır yüksekliği.
    pub image_max_rows: u16,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            images: ImageMode::Thumbnail,
            image_max_rows: 10,
        }
    }
}

/// Belirli bir boyutta çizilecek belirli bir resim.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ImageKey {
    pub url: String,
    pub cols: u16,
    pub rows: u16,
}

/// Denetleyiciye giden indirme+kodlama işi.
pub struct ImageJob {
    key: ImageKey,
    picker: Picker,
    origin: Option<(Snowflake, Snowflake)>,
}

impl ImageJob {
    pub fn url(&self) -> &str {
        &self.key.url
    }

    /// Resmin geldiği mesaj (kanal, mesaj); imzalı adresin süresi dolarsa yenilemek için.
    pub fn origin(&self) -> Option<(Snowflake, Snowflake)> {
        self.origin
    }

    /// Çözer, hedef hücre boyutuna ölçekler ve terminal protokolüne kodlar.
    /// Ağır iş: UI thread'inde çağrılmamalı.
    pub fn encode(self, bytes: &[u8]) -> UiEvent {
        let font = self.picker.font_size();
        let (px_w, px_h) = (
            u32::from(self.key.cols) * u32::from(font.width.max(1)),
            u32::from(self.key.rows) * u32::from(font.height.max(1)),
        );
        let result = digcord_media::decode_fit(bytes, px_w, px_h)
            .map_err(|e| e.to_string())
            .and_then(|img| {
                self.picker
                    .new_protocol(
                        img,
                        Size::new(self.key.cols, self.key.rows),
                        Resize::Fit(None),
                    )
                    .map_err(|e| e.to_string())
            });
        UiEvent::Image(ImageDone {
            key: self.key,
            result,
        })
    }

    /// İndirme başarısız oldu.
    pub fn failed(self, error: String) -> UiEvent {
        UiEvent::Image(ImageDone {
            key: self.key,
            result: Err(error),
        })
    }
}

impl fmt::Debug for ImageJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageJob").field("key", &self.key).finish()
    }
}

impl PartialEq for ImageJob {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for ImageJob {}

/// Bir resim işinin sonucu.
pub struct ImageDone {
    key: ImageKey,
    result: Result<Protocol, String>,
}

impl fmt::Debug for ImageDone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageDone")
            .field("key", &self.key)
            .field("ok", &self.result.is_ok())
            .finish()
    }
}

/// Bir resim yerinin o anki durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotState {
    Drawn,
    Loading,
    Failed,
}

pub(crate) struct Images {
    /// `None`: resimler kapalı.
    picker: Option<Picker>,
    mode: ImageMode,
    max_rows: u16,
    ready: RefCell<Lru<ImageKey, Protocol>>,
    failed: RefCell<HashSet<ImageKey>>,
    /// İstenmiş, sonucu beklenen.
    requested: RefCell<HashSet<ImageKey>>,
    /// Son çizimde istenen, henüz denetleyiciye gönderilmemiş işler.
    wanted: RefCell<Vec<ImageJob>>,
}

impl Images {
    pub(crate) fn new(picker: Option<Picker>, opts: Options) -> Self {
        Self {
            picker: picker.filter(|_| opts.images != ImageMode::Off),
            mode: opts.images,
            max_rows: opts.image_max_rows.max(1),
            ready: RefCell::new(Lru::new(CACHE_ENTRIES)),
            failed: RefCell::new(HashSet::new()),
            requested: RefCell::new(HashSet::new()),
            wanted: RefCell::new(Vec::new()),
        }
    }

    /// Kapalı (testler ve resimsiz kullanım).
    pub(crate) fn off() -> Self {
        Self::new(
            None,
            Options {
                images: ImageMode::Off,
                ..Options::default()
            },
        )
    }

    pub(crate) fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    /// Yarım blok yedeği mi (gerçek grafik protokolü yok).
    pub(crate) fn protocol_name(&self) -> Option<&'static str> {
        self.picker.as_ref().map(|p| match p.protocol_type() {
            ProtocolType::Halfblocks => "yarım blok",
            ProtocolType::Sixel => "sixel",
            ProtocolType::Kitty => "kitty",
            ProtocolType::Iterm2 => "iterm2",
        })
    }

    /// `w`×`h` piksellik resmin hücre boyutu: en fazla `max_cols` sütun ve `max_rows` satır,
    /// büyütme yok.
    pub(crate) fn fit(&self, w: u32, h: u32, max_cols: u16) -> Option<ImageFit> {
        self.fit_rows(w, h, max_cols, self.max_rows)
    }

    /// Tam ekran görüntüleyici için: verilen alana sığdır.
    pub(crate) fn fit_area(&self, w: u32, h: u32, area: Rect) -> Option<ImageFit> {
        let picker = self.picker.as_ref()?;
        fit_cells(picker, w, h, area.width, area.height)
    }

    fn fit_rows(&self, w: u32, h: u32, max_cols: u16, max_rows: u16) -> Option<ImageFit> {
        let picker = self.picker.as_ref()?;
        let max_cols = match self.mode {
            ImageMode::Off => return None,
            ImageMode::Thumbnail => max_cols.min(THUMBNAIL_COLS),
            ImageMode::Full => max_cols,
        };
        fit_cells(picker, w, h, max_cols, max_rows)
    }

    /// Resmi `area`'ya çizer; hazır değilse ister (`origin`: süresi dolan adresi yenilemek için).
    pub(crate) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        url: &str,
        origin: Option<(Snowflake, Snowflake)>,
    ) -> SlotState {
        let Some(picker) = &self.picker else {
            return SlotState::Failed;
        };
        let key = ImageKey {
            url: url.to_owned(),
            cols: area.width,
            rows: area.height,
        };
        if let Some(proto) = self.ready.borrow_mut().get(&key) {
            frame.render_widget(Image::new(proto), area);
            return SlotState::Drawn;
        }
        if self.failed.borrow().contains(&key) {
            return SlotState::Failed;
        }
        if self.requested.borrow_mut().insert(key.clone()) {
            self.wanted.borrow_mut().push(ImageJob {
                key,
                picker: picker.clone(),
                origin,
            });
        }
        SlotState::Loading
    }

    /// Çizim sırasında istenen işler.
    pub(crate) fn take_wanted(&self) -> Vec<ImageJob> {
        std::mem::take(&mut *self.wanted.borrow_mut())
    }

    pub(crate) fn done(&self, done: ImageDone) {
        self.requested.borrow_mut().remove(&done.key);
        match done.result {
            Ok(proto) => self.ready.borrow_mut().insert(done.key, proto),
            Err(e) => {
                tracing::debug!("resim yüklenemedi: {e}");
                self.failed.borrow_mut().insert(done.key);
            }
        }
    }
}

fn fit_cells(picker: &Picker, w: u32, h: u32, max_cols: u16, max_rows: u16) -> Option<ImageFit> {
    if w == 0 || h == 0 || max_cols == 0 || max_rows == 0 {
        return None;
    }
    let font = picker.font_size();
    let (fw, fh) = (f64::from(font.width.max(1)), f64::from(font.height.max(1)));
    let (w, h) = (f64::from(w), f64::from(h));
    let scale = (f64::from(max_cols) * fw / w)
        .min(f64::from(max_rows) * fh / h)
        .min(1.0);
    // Sonuçlar en fazla max_cols·fw ve max_rows·fh; u32/u16'ya sığar.
    let px_w = (w * scale).floor().max(1.0);
    let px_h = (h * scale).floor().max(1.0);
    Some(ImageFit {
        cols: ((px_w / fw).ceil() as u16).clamp(1, max_cols),
        rows: ((px_h / fh).ceil() as u16).clamp(1, max_rows),
        px_w: px_w as u32,
        px_h: px_h as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn images(mode: ImageMode, rows: u16) -> Images {
        Images::new(
            Some(Picker::halfblocks()),
            Options {
                images: mode,
                image_max_rows: rows,
            },
        )
    }

    #[test]
    fn fit_keeps_aspect_and_limits() {
        // Yarım blok yedeğinde hücre 10×20 piksel.
        let img = images(ImageMode::Full, 10);
        let f = img.fit(400, 200, 100).expect("fit");
        assert_eq!((f.cols, f.rows, f.px_w, f.px_h), (40, 10, 400, 200));
        // Satır sınırı: 12 satır = 240 px yükseklik.
        let f = img.fit(1000, 1000, 100).expect("fit");
        assert_eq!((f.cols, f.rows), (20, 10));
        // Küçük resim büyütülmez.
        let f = img.fit(48, 48, 100).expect("fit");
        assert_eq!((f.cols, f.rows, f.px_w), (5, 3, 48));
    }

    #[test]
    fn thumbnail_caps_width() {
        let f = images(ImageMode::Thumbnail, 50)
            .fit(4000, 500, 200)
            .expect("fit");
        assert_eq!(f.cols, THUMBNAIL_COLS);
    }

    #[test]
    fn off_means_no_fit() {
        assert!(images(ImageMode::Off, 10).fit(10, 10, 10).is_none());
        assert!(Images::off().fit(10, 10, 10).is_none());
        assert!(!Images::off().enabled());
    }

    #[test]
    fn request_once_then_draw_when_ready() {
        let img = images(ImageMode::Full, 10);
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).expect("term");
        let area = Rect::new(0, 0, 4, 2);
        let mut states = Vec::new();
        for _ in 0..2 {
            term.draw(|f| states.push(img.draw(f, area, "https://cdn.discordapp.com/x.png", None)))
                .expect("çiz");
        }
        assert_eq!(states, [SlotState::Loading, SlotState::Loading]);
        let jobs = img.take_wanted();
        assert_eq!(jobs.len(), 1, "aynı resim bir kez istenir");
        // 1×1 kırmızı PNG'yi kodla ve teslim et.
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("png");
        let job = jobs.into_iter().next().expect("iş");
        let UiEvent::Image(done) = job.encode(png.get_ref()) else {
            panic!("resim olayı bekleniyordu");
        };
        img.done(done);
        term.draw(|f| states.push(img.draw(f, area, "https://cdn.discordapp.com/x.png", None)))
            .expect("çiz");
        assert_eq!(states[2], SlotState::Drawn);
    }

    #[test]
    fn failure_is_remembered() {
        let img = images(ImageMode::Full, 10);
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 10)).expect("term");
        let area = Rect::new(0, 0, 4, 2);
        term.draw(|f| {
            img.draw(f, area, "u", None);
        })
        .expect("çiz");
        let job = img.take_wanted().into_iter().next().expect("iş");
        let UiEvent::Image(done) = job.failed("404".into()) else {
            panic!("resim olayı bekleniyordu");
        };
        img.done(done);
        let mut state = SlotState::Loading;
        term.draw(|f| state = img.draw(f, area, "u", None))
            .expect("çiz");
        assert_eq!(state, SlotState::Failed);
        assert!(img.take_wanted().is_empty());
    }
}
