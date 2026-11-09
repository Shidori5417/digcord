//! Ses G/Ç: Opus çözme/kodlama, kullanıcı başına jitter buffer'lı karıştırıcı ve cpal akışları.
//!
//! Karıştırıcı saf mantıktır (test edilir); cpal akışı ayrı bir thread'de yaşar (cpal
//! `Stream` her platformda `Send` değil) ve geri çağrıda karıştırıcıyı bloklamadan okur.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// 20 ms'de kanal başına örnek.
pub const FRAME_SAMPLES: usize = 960;
const FRAME_LEN: usize = FRAME_SAMPLES * CHANNELS;
/// Opus'un en uzun karesi (120 ms) için çözme tamponu.
const MAX_DECODED: usize = 5760 * CHANNELS;
/// Çalmaya bu kadar kare birikince başla (ağ titremesine karşı).
const JITTER_START_FRAMES: usize = 3;
/// Bundan fazlası birikirse en eskiler atılır (gecikme büyümesin).
const MAX_BUFFER_FRAMES: usize = 10;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("ses aygıtı yok")]
    NoDevice,
    #[error("ses akışı açılamadı: {0}")]
    Stream(String),
    #[error("opus: {0}")]
    Opus(#[from] opus::Error),
}

struct UserStream {
    decoder: opus::Decoder,
    buffer: VecDeque<f32>,
    playing: bool,
    volume: f32,
}

/// Gelen kullanıcı seslerini karıştırır.
pub struct Mixer {
    users: HashMap<u64, UserStream>,
    volumes: HashMap<u64, f32>,
    deafened: bool,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            users: HashMap::new(),
            volumes: HashMap::new(),
            deafened: false,
            scratch: vec![0.0; MAX_DECODED],
        }
    }
}

impl Mixer {
    /// Bir Opus karesini çözüp kullanıcının tamponuna ekler.
    pub fn push(&mut self, user: u64, opus: &[u8]) -> Result<(), AudioError> {
        let volume = self.volumes.get(&user).copied().unwrap_or(1.0);
        let stream = match self.users.entry(user) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(UserStream {
                decoder: opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo)?,
                buffer: VecDeque::with_capacity(FRAME_LEN * MAX_BUFFER_FRAMES),
                playing: false,
                volume,
            }),
        };
        let n = stream
            .decoder
            .decode_float(opus, &mut self.scratch, false)?;
        stream
            .buffer
            .extend(self.scratch[..n * CHANNELS].iter().copied());
        let max = FRAME_LEN * MAX_BUFFER_FRAMES;
        if stream.buffer.len() > max {
            let excess = stream.buffer.len() - max;
            stream.buffer.drain(..excess);
        }
        if !stream.playing && stream.buffer.len() >= FRAME_LEN * JITTER_START_FRAMES {
            stream.playing = true;
        }
        Ok(())
    }

    /// Kullanıcı başına ses seviyesi (0.0 = sessiz, 1.0 = normal, en fazla 2.0).
    pub fn set_volume(&mut self, user: u64, volume: f32) {
        let v = volume.clamp(0.0, 2.0);
        self.volumes.insert(user, v);
        if let Some(s) = self.users.get_mut(&user) {
            s.volume = v;
        }
    }

    /// Kulaklık kapalı: hiçbir şey çalınmaz (tamponlar boşaltılır).
    pub fn set_deafened(&mut self, deafened: bool) {
        self.deafened = deafened;
        if deafened {
            for s in self.users.values_mut() {
                s.buffer.clear();
                s.playing = false;
            }
        }
    }

    pub fn remove(&mut self, user: u64) {
        self.users.remove(&user);
    }

    /// Çıkış tamponunu (karışık stereo, iç içe) doldurur.
    pub fn fill(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        if self.deafened {
            return;
        }
        for s in self.users.values_mut() {
            if !s.playing {
                continue;
            }
            for o in out.iter_mut() {
                let Some(x) = s.buffer.pop_front() else {
                    // Tampon bitti: yeniden biriksin.
                    s.playing = false;
                    break;
                };
                *o += x * s.volume;
            }
        }
        for o in out.iter_mut() {
            *o = o.clamp(-1.0, 1.0);
        }
    }
}

/// Mikrofon PCM'ini 20 ms'lik Opus karelerine çevirir.
pub struct FrameEncoder {
    encoder: opus::Encoder,
    pending: Vec<f32>,
    out: Vec<u8>,
}

impl FrameEncoder {
    pub fn new() -> Result<Self, AudioError> {
        let mut encoder =
            opus::Encoder::new(SAMPLE_RATE, opus::Channels::Stereo, opus::Application::Voip)?;
        encoder.set_bitrate(opus::Bitrate::Bits(64_000))?;
        encoder.set_inband_fec(true)?;
        Ok(Self {
            encoder,
            pending: Vec::with_capacity(FRAME_LEN * 2),
            out: vec![0; 1500],
        })
    }

    /// Stereo iç içe örnekleri ekler; tamamlanan her kare için `emit` çağrılır.
    pub fn push(&mut self, samples: &[f32], mut emit: impl FnMut(&[u8])) -> Result<(), AudioError> {
        self.pending.extend_from_slice(samples);
        while self.pending.len() >= FRAME_LEN {
            let n = self
                .encoder
                .encode_float(&self.pending[..FRAME_LEN], &mut self.out)?;
            emit(&self.out[..n]);
            self.pending.drain(..FRAME_LEN);
        }
        Ok(())
    }
}

/// Karenin ortalama karekök seviyesi (0..1); konuşma algılama için.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Çalan bir cpal çıkış akışı; düşünce durur.
pub struct Playback {
    stop: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Bir cpal akışını kendi thread'inde açar; açılış sonucunu bekler.
fn spawn_stream(
    name: &str,
    build: impl FnOnce() -> Result<cpal::Stream, AudioError> + Send + 'static,
) -> Result<Playback, AudioError> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let stream = match build().and_then(|s| {
                s.play().map_err(|e| AudioError::Stream(e.to_string()))?;
                Ok(s)
            }) {
                Ok(s) => {
                    let _ = ready_tx.send(Ok(()));
                    s
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Gönderen düşene kadar bekle; akış bu thread'de yaşar.
            let _ = stop_rx.recv();
            drop(stream);
        })
        .map_err(|e| AudioError::Stream(e.to_string()))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(Playback {
            stop: Some(stop_tx),
            thread: Some(thread),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(AudioError::Stream("ses thread'i başlamadı".into())),
    }
}

fn stream_config(channels: u16) -> cpal::StreamConfig {
    cpal::StreamConfig {
        channels,
        sample_rate: SAMPLE_RATE,
        buffer_size: cpal::BufferSize::Default,
    }
}

/// Varsayılan çıkış aygıtında karıştırıcıyı çalar (48 kHz stereo f32).
pub fn start_playback(mixer: Arc<Mutex<Mixer>>) -> Result<Playback, AudioError> {
    spawn_stream("digcord-playback", move || {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or(AudioError::NoDevice)?;
        device
            .build_output_stream::<f32, _, _>(
                stream_config(CHANNELS as u16),
                move |data, _| match mixer.try_lock() {
                    // Ses thread'i asla beklemez: kilit meşgulse bu tur sessizlik.
                    Ok(mut m) => m.fill(data),
                    Err(_) => data.fill(0.0),
                },
                |e| tracing::warn!("ses çıkışı hatası: {e}"),
                None,
            )
            .map_err(|e| AudioError::Stream(e.to_string()))
    })
}

/// Varsayılan giriş aygıtından 48 kHz örnekleri stereo iç içe olarak `on_samples`'a verir.
pub fn start_capture(
    mut on_samples: impl FnMut(&[f32]) + Send + 'static,
) -> Result<Playback, AudioError> {
    spawn_stream("digcord-capture", move || {
        let device = cpal::default_host()
            .default_input_device()
            .ok_or(AudioError::NoDevice)?;
        let mut stereo = Vec::new();
        device
            .build_input_stream::<f32, _, _>(
                stream_config(1),
                move |data, _| {
                    stereo.clear();
                    for &x in data {
                        stereo.push(x);
                        stereo.push(x);
                    }
                    on_samples(&stereo);
                },
                |e| tracing::warn!("mikrofon hatası: {e}"),
                None,
            )
            .map_err(|e| AudioError::Stream(e.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_frames(n: usize, amp: f32) -> Vec<Vec<u8>> {
        let mut enc = FrameEncoder::new().expect("kodlayıcı");
        let mut pcm = Vec::with_capacity(FRAME_LEN * n);
        for i in 0..FRAME_SAMPLES * n {
            let x = amp * (i as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin();
            pcm.push(x);
            pcm.push(x);
        }
        let mut frames = Vec::new();
        enc.push(&pcm, |f| frames.push(f.to_vec())).expect("kodla");
        frames
    }

    fn level(m: &mut Mixer) -> f32 {
        let mut out = vec![0.0; FRAME_LEN];
        m.fill(&mut out);
        rms(&out)
    }

    #[test]
    fn encoder_emits_20ms_frames() {
        let frames = sine_frames(5, 0.5);
        assert_eq!(frames.len(), 5);
        assert!(frames.iter().all(|f| !f.is_empty() && f.len() < 400));
    }

    #[test]
    fn jitter_buffer_waits_then_plays() {
        let frames = sine_frames(6, 0.5);
        let mut m = Mixer::default();
        m.push(1, &frames[0]).expect("çöz");
        m.push(1, &frames[1]).expect("çöz");
        assert_eq!(level(&mut m), 0.0, "3 kare birikmeden çalma");
        m.push(1, &frames[2]).expect("çöz");
        // İlk kare kodlayıcı gecikmesi yüzünden sessiz olabilir; ikinciden itibaren ses var.
        level(&mut m);
        assert!(level(&mut m) > 0.1);
    }

    #[test]
    fn volume_deafen_and_mixing() {
        let frames = sine_frames(8, 0.3);
        // Aynı fazda iki kullanıcı ≈ iki kat seviye.
        let mut solo = Mixer::default();
        let mut duo = Mixer::default();
        for f in &frames {
            solo.push(1, f).expect("çöz");
            duo.push(1, f).expect("çöz");
            duo.push(2, f).expect("çöz");
        }
        level(&mut solo);
        level(&mut duo);
        let one = level(&mut solo);
        let two = level(&mut duo);
        assert!(two > one * 1.8, "iki kullanıcı toplanır: {one} → {two}");
        solo.set_volume(1, 0.0);
        assert_eq!(level(&mut solo), 0.0);
        duo.set_deafened(true);
        duo.push(1, &frames[0]).expect("çöz");
        assert_eq!(level(&mut duo), 0.0);
    }

    #[test]
    fn buffer_is_capped_and_garbage_rejected() {
        let frames = sine_frames(20, 0.3);
        let mut m = Mixer::default();
        for f in &frames {
            m.push(1, f).expect("çöz");
        }
        assert!(m.users[&1].buffer.len() <= FRAME_LEN * MAX_BUFFER_FRAMES);
        assert!(m.push(1, &[0xff; 3]).is_err() || m.push(1, &[]).is_err());
        assert_eq!(rms(&[]), 0.0);
    }

    /// Gerçek aygıt: varsayılan çıkışı sessiz karıştırıcıyla kısa süre açar (ses çalmaz).
    #[test]
    #[ignore = "ses aygıtı gerekir"]
    fn opens_default_output_silently() {
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let playback = start_playback(mixer).expect("çıkış akışı");
        std::thread::sleep(std::time::Duration::from_millis(500));
        drop(playback);
    }
}
