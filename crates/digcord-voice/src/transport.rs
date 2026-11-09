//! Taşıma şifrelemesi (UDP katmanı): `aead_aes256_gcm_rtpsize` ve
//! `aead_xchacha20_poly1305_rtpsize`. DAVE bunun **içinde** ikinci katmandır.
//!
//! Paket: RTP başlığı (AAD) | şifreli yük | etiket (16) | nonce sayacı (4, büyük endian).
//! AEAD nonce'u: sayaç (büyük endian) + sıfırlar (12 veya 24 bayt).
//! "rtpsize": AAD sabit başlık + CSRC'ler + uzantı başlığının 4 baytı; uzantı verisi
//! şifreli kısmın başındadır ve çözdükten sonra atılır.

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{AeadInOut, KeyInit};
use chacha20poly1305::XChaCha20Poly1305;

use crate::rtp;

const TAG_LEN: usize = 16;
const NONCE_SUFFIX: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Aes256Gcm,
    XChaCha20Poly1305,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Aes256Gcm => "aead_aes256_gcm_rtpsize",
            Self::XChaCha20Poly1305 => "aead_xchacha20_poly1305_rtpsize",
        }
    }

    /// Sunucunun sunduğu kiplerden tercih edilen (AES-GCM önce).
    pub fn choose<S: AsRef<str>>(offered: &[S]) -> Option<Self> {
        [Self::Aes256Gcm, Self::XChaCha20Poly1305]
            .into_iter()
            .find(|m| offered.iter().any(|o| o.as_ref() == m.name()))
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("geçersiz anahtar")]
    Key,
    #[error("bozuk paket")]
    Malformed,
    #[error("doğrulama başarısız")]
    Auth,
}

enum Cipher {
    Aes(Box<Aes256Gcm>),
    XChaCha(Box<XChaCha20Poly1305>),
}

/// Bir ses oturumunun taşıma şifrelemesi.
pub struct Transport {
    cipher: Cipher,
    /// Giden paket sayacı (nonce).
    counter: u32,
}

/// Çözülmüş gelen paket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub header: rtp::Parsed,
    /// Uzantı verisi atılmış yük (DAVE çerçevesi ya da Opus).
    pub payload: Vec<u8>,
}

impl Transport {
    pub fn new(mode: Mode, key: &[u8]) -> Result<Self, TransportError> {
        let cipher = match mode {
            Mode::Aes256Gcm => Cipher::Aes(Box::new(
                Aes256Gcm::new_from_slice(key).map_err(|_| TransportError::Key)?,
            )),
            Mode::XChaCha20Poly1305 => Cipher::XChaCha(Box::new(
                XChaCha20Poly1305::new_from_slice(key).map_err(|_| TransportError::Key)?,
            )),
        };
        Ok(Self { cipher, counter: 0 })
    }

    /// `header` + şifreli `payload` paketini `out`'a yazar (`out` temizlenir).
    pub fn seal(&mut self, header: &rtp::Header, payload: &[u8], out: &mut Vec<u8>) {
        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);
        out.clear();
        header.write(out);
        let aad_len = out.len();
        out.extend_from_slice(payload);
        let (aad, body) = out.split_at_mut(aad_len);
        let tag = match &self.cipher {
            Cipher::Aes(c) => {
                let nonce = nonce_bytes::<12>(counter);
                c.encrypt_inout_detached(&nonce.into(), aad, body.into())
                    .map(|t| t.to_vec())
            }
            Cipher::XChaCha(c) => {
                let nonce = nonce_bytes::<24>(counter);
                c.encrypt_inout_detached(&nonce.into(), aad, body.into())
                    .map(|t| t.to_vec())
            }
        };
        // Sadece aşırı uzun girdide hata olur; ses karesi bunun çok altında.
        match tag {
            Ok(tag) => out.extend_from_slice(&tag),
            Err(_) => {
                out.clear();
                return;
            }
        }
        out.extend_from_slice(&counter.to_be_bytes());
    }

    /// Gelen paketi doğrular ve çözer.
    pub fn open(&self, packet: &[u8]) -> Result<Opened, TransportError> {
        let header = rtp::parse(packet).ok_or(TransportError::Malformed)?;
        if packet.len() < header.aad_len + TAG_LEN + NONCE_SUFFIX {
            return Err(TransportError::Malformed);
        }
        let (rest, suffix) = packet.split_at(packet.len() - NONCE_SUFFIX);
        let counter = u32::from_be_bytes([suffix[0], suffix[1], suffix[2], suffix[3]]);
        let (rest, tag) = rest.split_at(rest.len() - TAG_LEN);
        let (aad, ciphertext) = rest.split_at(header.aad_len);
        let mut body = ciphertext.to_vec();
        let ok = match &self.cipher {
            Cipher::Aes(c) => {
                let nonce = nonce_bytes::<12>(counter);
                let tag = aes_gcm::Tag::try_from(tag).map_err(|_| TransportError::Malformed)?;
                c.decrypt_inout_detached(&nonce.into(), aad, body.as_mut_slice().into(), &tag)
            }
            Cipher::XChaCha(c) => {
                let nonce = nonce_bytes::<24>(counter);
                let tag =
                    chacha20poly1305::Tag::try_from(tag).map_err(|_| TransportError::Malformed)?;
                c.decrypt_inout_detached(&nonce.into(), aad, body.as_mut_slice().into(), &tag)
            }
        };
        ok.map_err(|_| TransportError::Auth)?;
        if body.len() < header.extension_len {
            return Err(TransportError::Malformed);
        }
        body.drain(..header.extension_len);
        Ok(Opened {
            header,
            payload: body,
        })
    }
}

fn nonce_bytes<const N: usize>(counter: u32) -> [u8; N] {
    let mut n = [0u8; N];
    n[..4].copy_from_slice(&counter.to_be_bytes());
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7; 32];
    const HEADER: rtp::Header = rtp::Header {
        sequence: 5,
        timestamp: 960,
        ssrc: 42,
    };

    #[test]
    fn choose_prefers_aes() {
        let offered = [
            "aead_xchacha20_poly1305_rtpsize",
            "aead_aes256_gcm_rtpsize",
            "xsalsa20_poly1305",
        ];
        assert_eq!(Mode::choose(&offered), Some(Mode::Aes256Gcm));
        assert_eq!(
            Mode::choose(&["aead_xchacha20_poly1305_rtpsize"]),
            Some(Mode::XChaCha20Poly1305)
        );
        assert_eq!(Mode::choose(&["xsalsa20_poly1305"]), None);
    }

    #[test]
    fn roundtrip_both_modes_and_layout() {
        for mode in [Mode::Aes256Gcm, Mode::XChaCha20Poly1305] {
            let mut tx = Transport::new(mode, &KEY).expect("anahtar");
            let rx = Transport::new(mode, &KEY).expect("anahtar");
            let mut out = Vec::new();
            for i in 0..3u32 {
                tx.seal(&HEADER, b"opus karesi", &mut out);
                assert_eq!(out.len(), 12 + 11 + 16 + 4);
                assert_eq!(&out[..12], &[0x80, 0x78, 0, 5, 0, 0, 3, 0xC0, 0, 0, 0, 42]);
                assert_eq!(&out[out.len() - 4..], &i.to_be_bytes(), "sayaç sonda");
                assert_ne!(&out[12..23], b"opus karesi", "yük şifreli");
                let opened = rx.open(&out).expect("çöz");
                assert_eq!(opened.payload, b"opus karesi");
                assert_eq!(opened.header.ssrc, 42);
            }
        }
    }

    #[test]
    fn known_answer_aes_gcm_nonce_is_counter_prefix() {
        // Aynı anahtar/nonce/AAD ile doğrudan AES-256-GCM sonucu paketle aynı olmalı.
        let mut tx = Transport::new(Mode::Aes256Gcm, &KEY).expect("anahtar");
        let mut out = Vec::new();
        tx.seal(&HEADER, b"x", &mut out);
        tx.seal(&HEADER, b"x", &mut out); // sayaç = 1
        let c = Aes256Gcm::new_from_slice(&KEY).expect("anahtar");
        let mut nonce = [0u8; 12];
        nonce[3] = 1;
        let mut body = *b"x";
        let tag = c
            .encrypt_inout_detached(&nonce.into(), &out[..12], body.as_mut_slice().into())
            .expect("şifrele");
        assert_eq!(out[12], body[0]);
        assert_eq!(&out[13..29], tag.as_slice());
    }

    #[test]
    fn tamper_and_wrong_key_fail() {
        let mut tx = Transport::new(Mode::Aes256Gcm, &KEY).expect("anahtar");
        let mut out = Vec::new();
        tx.seal(&HEADER, b"merhaba", &mut out);
        let rx = Transport::new(Mode::Aes256Gcm, &KEY).expect("anahtar");
        let mut bad = out.clone();
        bad[2] ^= 1; // AAD (sıra numarası)
        assert_eq!(rx.open(&bad), Err(TransportError::Auth));
        let other = Transport::new(Mode::Aes256Gcm, &[8; 32]).expect("anahtar");
        assert_eq!(other.open(&out), Err(TransportError::Auth));
        assert_eq!(rx.open(&out[..20]), Err(TransportError::Malformed));
        assert!(Transport::new(Mode::Aes256Gcm, &[0; 16]).is_err());
    }

    #[test]
    fn extension_body_is_encrypted_and_stripped() {
        // Uzantılı gelen paketi elle kur: başlık(12)+uzantı başlığı(4) AAD, uzantı verisi(4)+yük şifreli.
        let key = [3u8; 32];
        let c = Aes256Gcm::new_from_slice(&key).expect("anahtar");
        let mut pkt = vec![0x90, 0x78, 0, 1, 0, 0, 0, 0, 0, 0, 0, 9, 0xBE, 0xDE, 0, 1];
        let mut body = vec![0xAA, 0xBB, 0xCC, 0xDD];
        body.extend_from_slice(b"ses");
        let nonce = nonce_bytes::<12>(77);
        let tag = c
            .encrypt_inout_detached(&nonce.into(), &pkt, body.as_mut_slice().into())
            .expect("şifrele");
        pkt.extend_from_slice(&body);
        pkt.extend_from_slice(&tag);
        pkt.extend_from_slice(&77u32.to_be_bytes());
        let rx = Transport::new(Mode::Aes256Gcm, &key).expect("anahtar");
        let opened = rx.open(&pkt).expect("çöz");
        assert_eq!(opened.payload, b"ses");
        assert_eq!(opened.header.ssrc, 9);
    }
}
