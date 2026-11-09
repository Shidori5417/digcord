//! Ses yolunun kare başına maliyeti: 1 gönderen (kodla + DAVE + taşıma) ve 3 konuşan dinleme
//! (taşıma + DAVE çöz + Opus çöz + karıştır). 20 ms'lik kare süresine oranı yazdırılır.
//! Çalıştır: `cargo test --release -p digcord-voice --test cpu_budget -- --ignored --nocapture`

use std::time::{Duration, Instant};

use digcord_voice::audio::{CHANNELS, FRAME_SAMPLES, FrameEncoder, Mixer};
use digcord_voice::dave::cryptor::{Decryptor, Encryptor};
use digcord_voice::rtp;
use digcord_voice::transport::{Mode, Transport};

#[test]
#[ignore = "ölçüm; --release ile çalıştırılmalı"]
fn audio_path_cpu_budget() {
    const FRAMES: usize = 1500; // 30 sn
    const SPEAKERS: usize = 3;
    let key = [5u8; 32];
    let mut enc = FrameEncoder::new().expect("kodlayıcı");
    let mut dave_tx = Encryptor::default();
    dave_tx.set_ratchet(&[1; 16]);
    let mut transport_tx = Transport::new(Mode::Aes256Gcm, &key).expect("anahtar");
    let transport_rx = Transport::new(Mode::Aes256Gcm, &key).expect("anahtar");
    let start = Instant::now();
    let mut dave_rx: Vec<Decryptor> = (0..SPEAKERS)
        .map(|_| {
            let mut d = Decryptor::default();
            d.add_ratchet(&[1; 16], start);
            d
        })
        .collect();
    let mut mixer = Mixer::default();
    let mut out = vec![0.0f32; FRAME_SAMPLES * CHANNELS];
    let mut pcm = vec![0.0f32; FRAME_SAMPLES * CHANNELS];
    let (mut frame, mut packet) = (Vec::new(), Vec::new());

    let t0 = Instant::now();
    for i in 0..FRAMES {
        for (n, s) in pcm.iter_mut().enumerate() {
            *s = 0.3 * ((i * FRAME_SAMPLES + n / 2) as f32 * 0.0577).sin();
        }
        let mut opus = Vec::new();
        enc.push(&pcm, |f| opus = f.to_vec()).expect("kodla");
        assert!(dave_tx.encrypt(&opus, &mut frame));
        let header = rtp::Header {
            sequence: i as u16,
            timestamp: (i * FRAME_SAMPLES) as u32,
            ssrc: 1,
        };
        transport_tx.seal(&header, &frame, &mut packet);
        // Aynı paketi 3 farklı konuşandan geliyormuş gibi işle (her çözücü kendi penceresinde).
        for (u, d) in dave_rx.iter_mut().enumerate() {
            let opened = transport_rx.open(&packet).expect("aç");
            let plain = d
                .decrypt(&opened.payload, start + Duration::from_secs(1))
                .expect("DAVE çöz");
            mixer.push(u as u64, &plain).expect("opus çöz");
        }
        mixer.fill(&mut out);
    }
    let per_frame = t0.elapsed() / FRAMES as u32;
    let percent = per_frame.as_secs_f64() / 0.020 * 100.0;
    println!("kare başına {per_frame:?} → tek çekirdeğin %{percent:.2}'si");
    assert!(percent < 3.0, "ses yolu bütçeyi aşıyor: %{percent:.2}");
}
