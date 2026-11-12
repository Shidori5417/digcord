//! Dosyaya log yazma ve token maskeleme.
//!
//! Log `~/.local/state/digcord/digcord.log` dosyasına, ayrı bir thread üzerinden
//! (`tracing_appender::non_blocking`) yazılır; UI thread'i diske beklemez.
//! Yazılan her satırdaki token benzeri dizeler `[TOKEN]` ile değiştirilir.

use std::borrow::Cow;
use std::io::{self, Write};

use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::{FilterExt as _, LevelFilter, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::prelude::*;

const MASK: &str = "[TOKEN]";

/// `init`in döndürdüğü guard'lar: düşürülünce bekleyen log ve teşhis satırları yazılır.
pub struct Guards {
    _log: WorkerGuard,
    _diag: Option<crate::diag::DiagGuard>,
}

/// Log altyapısını kurar (`digcord.log` + token içermeyen `diag.log`). Dönen guard'lar
/// `main` sonuna kadar tutulmalı.
pub fn init() -> anyhow::Result<Guards> {
    let dir = dirs::state_dir()
        .context("XDG state dizini bulunamadı")?
        .join("digcord");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("log dizini oluşturulamadı: {}", dir.display()))?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::NEVER)
        .filename_prefix("digcord.log")
        .build(&dir)
        .context("log dosyası açılamadı")?;
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let level = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    // Teşhis olayları (`target: "diag"`) sadece diag.log'a gider; açılamazsa uygulama sürer.
    let (diag_layer, diag_guard) = match crate::diag::default_path().map(crate::diag::init) {
        Some(Ok((layer, guard))) => (Some(layer), Some(guard)),
        _ => (None, None),
    };
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(MaskingMakeWriter(writer))
        .with_filter(
            LevelFilter::from_level(level).and(filter_fn(|m| m.target() != crate::diag::TARGET)),
        );
    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(diag_layer)
        .init();
    if diag_guard.is_none() {
        tracing::warn!("teşhis günlüğü (diag.log) açılamadı; devam ediliyor");
    }
    Ok(Guards {
        _log: guard,
        _diag: diag_guard,
    })
}

/// Ürettiği her yazıcıyı [`MaskingWriter`] ile saran `MakeWriter`.
pub struct MaskingMakeWriter<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for MaskingMakeWriter<M> {
    type Writer = MaskingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        MaskingWriter(self.0.make_writer())
    }
}

/// Yazılan metindeki token'ları maskeleyip iç yazıcıya aktarır.
///
/// `tracing_subscriber::fmt` her olayı önce tek bir tampona biçimlendirip tek
/// `write_all` çağrısıyla yazar; bu yüzden bir token iki `write` çağrısına bölünmez.
pub struct MaskingWriter<W>(W);

impl<W: Write> Write for MaskingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        self.0.write_all(mask_tokens(&text).as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Discord token'ına benzeyen dizeleri `[TOKEN]` ile değiştirir.
///
/// Yakalanan biçimler:
/// - `<base64 kullanıcı id>.<base64 zaman>.<hmac>` (ör. 24+.6.27+ karakter)
/// - eski `mfa.<uzun dize>` biçimi
///
/// Şüpheli durumda fazla maskelemek tercih edilir.
pub fn mask_tokens(text: &str) -> Cow<'_, str> {
    let mut out: Option<String> = None;
    let mut last = 0;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if !is_token_byte(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_token_byte(bytes[i]) {
            i += 1;
        }
        let run = &text[start..i];
        if looks_like_token(run) {
            let s = out.get_or_insert_with(|| String::with_capacity(text.len()));
            s.push_str(&text[last..start]);
            s.push_str(MASK);
            last = i;
        }
    }
    match out {
        Some(mut s) => {
            s.push_str(&text[last..]);
            Cow::Owned(s)
        }
        None => Cow::Borrowed(text),
    }
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')
}

fn looks_like_token(run: &str) -> bool {
    if let Some(rest) = run.strip_prefix("mfa.") {
        return rest.len() >= 20;
    }
    let parts: Vec<&str> = run.split('.').collect();
    parts
        .windows(3)
        .any(|w| w[0].len() >= 18 && (5..=8).contains(&w[1].len()) && w[2].len() >= 20)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // Gerçek token değil; sadece biçimi taklit eden sahte değer.
    const FAKE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAA.BBBBBB.CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";

    #[test]
    fn masks_standard_token() {
        let line = format!("Authorization: {FAKE} gönderildi");
        assert_eq!(mask_tokens(&line), "Authorization: [TOKEN] gönderildi");
    }

    #[test]
    fn masks_mfa_token() {
        let line = "token=mfa.ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef-_123";
        assert_eq!(mask_tokens(line), "token=[TOKEN]");
    }

    #[test]
    fn masks_multiple_and_quoted() {
        let line = format!("\"{FAKE}\" ve '{FAKE}'");
        assert_eq!(mask_tokens(&line), "\"[TOKEN]\" ve '[TOKEN]'");
    }

    #[test]
    fn leaves_normal_text_untouched() {
        for line in [
            "gateway bağlandı, seq=42",
            "https://discord.com/api/v9/channels/123456789012345678/messages",
            "sürüm 0.1.0, ör. e.g. a.b.c",
            "",
        ] {
            assert!(matches!(mask_tokens(line), Cow::Borrowed(_)), "{line}");
        }
    }

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl Write for Buf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("test kilidi").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn subscriber_output_is_masked() {
        let buf = Buf::default();
        let sink = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(MaskingMakeWriter(move || sink.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(token = FAKE, "giriş denendi: {FAKE}");
        });
        let out = String::from_utf8(buf.0.lock().expect("test kilidi").clone()).expect("utf8");
        assert!(!out.contains(FAKE), "{out}");
        assert_eq!(out.matches(MASK).count(), 2, "{out}");
    }
}
