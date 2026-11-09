//! Açık ses oturumu: ses bağlantısı görevi, çalma ve mikrofon akışları, ses algılama (VAD).
//!
//! Yüksek hızlı ses olayları (kareler) denetleyiciden geçmez: gelen kareler doğrudan
//! karıştırıcıya, mikrofon kareleri doğrudan bağlantıya gider. Durum olayları (bağlandı,
//! konuşuyor, kapandı) `on_event` ile denetleyiciye döner.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use digcord_voice::audio::{self, FrameEncoder, Mixer, Playback};
use digcord_voice::connection::{self, Command, Config, Event};
use digcord_voice::dave::frame::OPUS_SILENCE;
use digcord_voice::protocol::SessionInfo;
use tokio::sync::{mpsc, watch};

/// Konuşma bittikten sonra bekleme (kare; 20 ms). Eşik (RMS) `Options::vad_threshold`'dan gelir.
const VAD_HANGOVER_FRAMES: u32 = 15;
/// Konuşma bitince gönderilen sessizlik kareleri (belgelenmiş).
const SILENCE_FRAMES: usize = 5;

/// Ses oturumu ayarları.
#[derive(Debug, Clone)]
pub struct Options {
    pub muted: bool,
    pub deafened: bool,
    /// Bas-konuş: sadece tuş basılıyken gönder (yoksa ses algılama).
    pub push_to_talk: bool,
    pub jitter_frames: usize,
    /// Ses algılama eşiği (RMS).
    pub vad_threshold: f32,
    /// cpal aygıt adları; `None` = sistem varsayılanı.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

pub struct VoiceSession {
    stop: watch::Sender<bool>,
    muted: Arc<AtomicBool>,
    /// Ses algılama eşiği (`f32` bitleri); çalışırken değiştirilebilir.
    vad_threshold: Arc<AtomicU32>,
    /// Bas-konuş tuşu basılı mı (bas-konuş kapalıysa `None`).
    ptt: Option<Arc<AtomicBool>>,
    mixer: Arc<Mutex<Mixer>>,
    _playback: Option<Playback>,
    _capture: Option<Playback>,
}

impl VoiceSession {
    /// Bağlantı görevini ve ses akışlarını başlatır. `on_event` tokio görevinden çağrılır.
    pub fn start(
        info: SessionInfo,
        channel_id: u64,
        user_agent: String,
        opts: Options,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Self {
        let (stop, stop_rx) = watch::channel(false);
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(64);
        let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(256);
        let mixer = Arc::new(Mutex::new(Mixer::with_jitter(opts.jitter_frames)));
        if let Ok(mut m) = mixer.lock() {
            m.set_deafened(opts.deafened);
        }
        let muted = Arc::new(AtomicBool::new(opts.muted));
        let vad_threshold = Arc::new(AtomicU32::new(opts.vad_threshold.to_bits()));
        let ptt = opts.push_to_talk.then(|| Arc::new(AtomicBool::new(false)));

        tokio::spawn(connection::run(
            Config {
                info,
                channel_id,
                user_agent,
                url_override: None,
            },
            ev_tx,
            cmd_rx,
            stop_rx,
        ));
        let mixer_rx = mixer.clone();
        tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                match ev {
                    Event::Audio { user_id, opus } => {
                        if let Ok(mut m) = mixer_rx.lock()
                            && let Err(e) = m.push(user_id, &opus)
                        {
                            tracing::debug!("ses karesi çözülemedi: {e}");
                        }
                    }
                    Event::UserLeft(user) => {
                        if let Ok(mut m) = mixer_rx.lock() {
                            m.remove(user);
                        }
                        on_event(Event::UserLeft(user));
                    }
                    other => on_event(other),
                }
            }
        });

        let playback = audio::start_playback(mixer.clone(), opts.output_device)
            .map_err(|e| tracing::warn!("ses çıkışı açılamadı: {e}"))
            .ok();
        let capture = start_capture(
            cmd_tx,
            muted.clone(),
            ptt.clone(),
            vad_threshold.clone(),
            opts.input_device,
        );
        Self {
            stop,
            muted,
            vad_threshold,
            ptt,
            mixer,
            _playback: playback,
            _capture: capture,
        }
    }

    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Ses algılama eşiğini (RMS) anında değiştirir.
    pub fn set_vad_threshold(&self, threshold: f32) {
        self.vad_threshold
            .store(threshold.to_bits(), Ordering::Relaxed);
    }

    /// Bas-konuş tuşu durumu (bas-konuş kapalıysa etkisiz).
    pub fn set_push_to_talk(&self, pressed: bool) {
        if let Some(p) = &self.ptt {
            p.store(pressed, Ordering::Relaxed);
        }
    }

    pub fn set_deafened(&self, deafened: bool) {
        if let Ok(mut m) = self.mixer.lock() {
            m.set_deafened(deafened);
        }
    }

    pub fn set_volume(&self, user: u64, volume: f32) {
        if let Ok(mut m) = self.mixer.lock() {
            m.set_volume(user, volume);
        }
    }

    /// Bağlantıyı kapatır; akışlar düşünce durur.
    pub fn stop(self) {
        let _ = self.stop.send(true);
    }
}

/// Mikrofon → Opus → ses algılama → bağlantı. Ses thread'inde çalışır; asla beklemez.
fn start_capture(
    commands: mpsc::Sender<Command>,
    muted: Arc<AtomicBool>,
    ptt: Option<Arc<AtomicBool>>,
    threshold: Arc<AtomicU32>,
    device: Option<String>,
) -> Option<Playback> {
    let mut encoder = match FrameEncoder::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Opus kodlayıcı kurulamadı: {e}");
            return None;
        }
    };
    let mut vad = Vad::default();
    let mut frame_pcm: Vec<f32> = Vec::new();
    audio::start_capture(
        move |samples| {
            // Kare seviyesi için PCM'i kare kare izle (kodlayıcı da 20 ms'de bir yayınlar).
            frame_pcm.extend_from_slice(samples);
            let level = audio::rms(&frame_pcm);
            if frame_pcm.len() >= audio::FRAME_SAMPLES * audio::CHANNELS {
                frame_pcm.clear();
            }
            let silent_mic = muted.load(Ordering::Relaxed);
            // Bas-konuş: tuş basılıyken eşik aşılmış sayılır, değilken hiç.
            let level = match &ptt {
                Some(p) if p.load(Ordering::Relaxed) => 1.0,
                Some(_) => 0.0,
                None => level,
            };
            let limit = f32::from_bits(threshold.load(Ordering::Relaxed));
            let _ = encoder.push(samples, |opus| match vad.step(level, silent_mic, limit) {
                VadOutput::Send => {
                    let _ = commands.try_send(Command::Opus(opus.to_vec()));
                }
                VadOutput::Stop => {
                    for _ in 0..SILENCE_FRAMES {
                        let _ = commands.try_send(Command::Opus(OPUS_SILENCE.to_vec()));
                    }
                    let _ = commands.try_send(Command::StopSpeaking);
                }
                VadOutput::Quiet => {}
            });
        },
        device,
    )
    .map_err(|e| tracing::warn!("mikrofon açılamadı: {e}"))
    .ok()
}

#[derive(Debug, PartialEq, Eq)]
enum VadOutput {
    Send,
    Stop,
    Quiet,
}

/// Basit enerji eşiğiyle konuşma algılama; susturulunca konuşma kesilir.
#[derive(Default)]
struct Vad {
    active: bool,
    quiet_frames: u32,
}

impl Vad {
    fn step(&mut self, level: f32, muted: bool, threshold: f32) -> VadOutput {
        let loud = !muted && level >= threshold;
        if loud {
            self.active = true;
            self.quiet_frames = 0;
            return VadOutput::Send;
        }
        if !self.active {
            return VadOutput::Quiet;
        }
        self.quiet_frames += 1;
        if muted || self.quiet_frames > VAD_HANGOVER_FRAMES {
            self.active = false;
            return VadOutput::Stop;
        }
        VadOutput::Send
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: f32 = 0.012;

    #[test]
    fn vad_hangover_and_mute() {
        let mut v = Vad::default();
        assert_eq!(v.step(0.0, false, T), VadOutput::Quiet);
        assert_eq!(v.step(0.1, false, T), VadOutput::Send);
        for _ in 0..VAD_HANGOVER_FRAMES {
            assert_eq!(
                v.step(0.0, false, T),
                VadOutput::Send,
                "kısa duraklamada kesilmez"
            );
        }
        assert_eq!(v.step(0.0, false, T), VadOutput::Stop);
        assert_eq!(v.step(0.0, false, T), VadOutput::Quiet);
        assert_eq!(v.step(0.1, false, T), VadOutput::Send);
        assert_eq!(
            v.step(0.1, true, T),
            VadOutput::Stop,
            "susturunca hemen biter"
        );
        assert_eq!(v.step(0.5, true, T), VadOutput::Quiet);
    }

    #[test]
    fn vad_threshold_decides_loudness() {
        let mut v = Vad::default();
        assert_eq!(v.step(0.05, false, 0.1), VadOutput::Quiet, "eşiğin altında");
        assert_eq!(v.step(0.05, false, 0.01), VadOutput::Send, "eşiğin üstünde");
        let mut v = Vad::default();
        assert_eq!(v.step(0.1, false, 0.1), VadOutput::Send, "eşit sayılır");
    }
}
