//! Gönderen şifreleyici ve kullanıcı başına çözücü (libdave "CryptorManager" kuralları).
//!
//! - Nonce (u32) üst baytı nesil, alt 24 bit nesil içi sayaç; nesil baytı 256'da sarar,
//!   gerçek nesil [`wrapped_generation`] ile bulunur.
//! - Yeni epoch anahtarı gelince eskiler 10 sn daha geçerli (geçiş sırasında gelen kareler).
//! - Tekrar koruması: (nesil, nonce) çifti en fazla bir kez çözülür; geç gelen kareler için
//!   1000'lik eksik listesi.
//! - Zaman parametre olarak alınır (test edilebilirlik).

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use super::frame::{self, OPUS_SILENCE};
use super::ratchet::{HashRatchet, KEY_LEN};

/// Eski anahtar/nesil bu kadar süre daha kullanılabilir.
pub const CRYPTOR_EXPIRY: Duration = Duration::from_secs(10);
const MAX_GENERATION_GAP: u32 = 250;
/// Saniyede en fazla kare (ses 50 + video 2×60); zincirin zorla ilerletilmesini sınırlar.
const MAX_FRAMES_PER_SECOND: u64 = 170;
const MAX_MISSING_NONCES: usize = 1000;

/// Nesil baytından (nonce >> 24) gerçek nesli bulur.
pub fn wrapped_generation(oldest: u32, generation_byte: u32) -> u32 {
    let remainder = oldest % 256;
    let factor = oldest / 256 + u32::from(generation_byte < remainder);
    factor.wrapping_mul(256).wrapping_add(generation_byte)
}

/// Kendi karelerimizi şifreler.
#[derive(Default)]
pub struct Encryptor {
    ratchet: Option<(Vec<u8>, HashRatchet)>,
    nonce: u32,
    generation: u32,
    key: Option<[u8; KEY_LEN]>,
    passthrough: bool,
}

impl Encryptor {
    /// Yeni epoch'un kendi taban sırrı. Aynı sır tekrar verilirse sayaç sıfırlanmaz.
    pub fn set_ratchet(&mut self, base_secret: &[u8]) {
        if self.ratchet.as_ref().is_some_and(|(b, _)| b == base_secret) {
            return;
        }
        self.ratchet = Some((base_secret.to_vec(), HashRatchet::new(base_secret)));
        self.nonce = 0;
        self.generation = 0;
        self.key = None;
        self.passthrough = false;
    }

    /// DAVE'siz (sürüm 0) çağrı: kareler şifresiz gider.
    pub fn set_passthrough(&mut self) {
        self.ratchet = None;
        self.key = None;
        self.passthrough = true;
    }

    /// Şifreleme mümkün mü (anahtar ya da şifresiz geçiş var).
    pub fn ready(&self) -> bool {
        self.passthrough || self.ratchet.is_some()
    }

    /// Opus karesini gönderilecek biçime çevirir; anahtar yoksa `false` (kare atılmalı).
    pub fn encrypt(&mut self, opus: &[u8], out: &mut Vec<u8>) -> bool {
        if self.passthrough || opus == OPUS_SILENCE {
            out.clear();
            out.extend_from_slice(opus);
            return true;
        }
        let Some((_, ratchet)) = self.ratchet.as_mut() else {
            return false;
        };
        self.nonce = self.nonce.wrapping_add(1);
        let generation = wrapped_generation(self.generation, self.nonce >> 24);
        if generation != self.generation || self.key.is_none() {
            let Some(key) = ratchet.get(generation) else {
                return false;
            };
            for old in self.generation..generation {
                ratchet.erase(old);
            }
            self.generation = generation;
            self.key = Some(key);
        }
        let Some(key) = &self.key else { return false };
        frame::encrypt(key, self.nonce, opus, out);
        !out.is_empty()
    }
}

/// Bir epoch'un (taban sırrın) anahtarlarıyla çözme durumu.
struct Manager {
    base: Vec<u8>,
    ratchet: HashRatchet,
    created: Instant,
    expiry: Option<Instant>,
    oldest_generation: u32,
    newest_generation: u32,
    /// nesil → (anahtar, son kullanma)
    keys: BTreeMap<u32, ([u8; KEY_LEN], Option<Instant>)>,
    newest_nonce: Option<u64>,
    missing: VecDeque<u64>,
}

impl Manager {
    fn new(base: &[u8], now: Instant) -> Self {
        Self {
            base: base.to_vec(),
            ratchet: HashRatchet::new(base),
            created: now,
            expiry: None,
            oldest_generation: 0,
            newest_generation: 0,
            keys: BTreeMap::new(),
            newest_nonce: None,
            missing: VecDeque::new(),
        }
    }

    fn expired(&self, now: Instant) -> bool {
        self.expiry.is_some_and(|e| now >= e)
    }

    fn drop_expired_keys(&mut self, now: Instant) {
        let expired: Vec<u32> = self
            .keys
            .iter()
            .filter(|(_, (_, e))| e.is_some_and(|e| now >= e))
            .map(|(g, _)| *g)
            .collect();
        for g in expired {
            self.keys.remove(&g);
            self.ratchet.erase(g);
        }
        // En eski kullanılabilir nesil: kalan en küçük, yoksa en yeni.
        let oldest = self
            .keys
            .keys()
            .next()
            .copied()
            .unwrap_or(self.newest_generation);
        self.oldest_generation = self
            .oldest_generation
            .max(oldest.min(self.newest_generation));
    }

    fn try_decrypt(&mut self, p: &frame::Protected<'_>, now: Instant) -> Option<Vec<u8>> {
        self.drop_expired_keys(now);
        let generation = wrapped_generation(self.oldest_generation, p.nonce >> 24);
        if generation < self.oldest_generation
            || generation > self.newest_generation.saturating_add(MAX_GENERATION_GAP)
        {
            return None;
        }
        let secs = now.saturating_duration_since(self.created).as_secs();
        if u64::from(generation) > (MAX_FRAMES_PER_SECOND.saturating_mul(secs)) >> 24 {
            return None;
        }
        let big = (u64::from(generation) << 24) | u64::from(p.nonce & 0x00FF_FFFF);
        let expected = self.newest_nonce.map_or(0, |n| n + 1);
        if big < expected && !self.missing.contains(&big) {
            return None;
        }
        let key = match self.keys.get(&generation) {
            Some((k, _)) => *k,
            None => {
                let k = self.ratchet.get(generation)?;
                // Yeni nesil daha eskiyse hemen süreli (geç gelen kareler için).
                let expiry = (generation < self.newest_generation).then(|| now + CRYPTOR_EXPIRY);
                self.keys.insert(generation, (k, expiry));
                k
            }
        };
        let plain = p.decrypt(&key)?;

        // Tekrar penceresi.
        if big >= expected {
            let skipped = (big - expected).min(MAX_MISSING_NONCES as u64);
            for n in big - skipped..big {
                self.missing.push_back(n);
            }
            while self.missing.len() > MAX_MISSING_NONCES {
                self.missing.pop_front();
            }
            self.newest_nonce = Some(big);
        } else if let Some(i) = self.missing.iter().position(|n| *n == big) {
            self.missing.remove(i);
        }
        // Daha yeni nesil başarıyla çözüldü: eski nesiller süreli.
        if generation > self.newest_generation {
            self.newest_generation = generation;
            for (g, (_, e)) in self.keys.iter_mut() {
                if *g < generation && e.is_none() {
                    *e = Some(now + CRYPTOR_EXPIRY);
                }
            }
        }
        Some(plain)
    }
}

/// Bir kullanıcının karelerini çözer.
#[derive(Default)]
pub struct Decryptor {
    /// En yeni sonda.
    managers: VecDeque<Manager>,
    passthrough_forever: bool,
    passthrough_until: Option<Instant>,
}

impl Decryptor {
    /// Yeni epoch'un taban sırrı; eski epoch'lar `CRYPTOR_EXPIRY` daha geçerli.
    pub fn add_ratchet(&mut self, base_secret: &[u8], now: Instant) {
        self.managers.retain(|m| !m.expired(now));
        if self.managers.iter().any(|m| m.base == base_secret) {
            return;
        }
        for m in &mut self.managers {
            m.expiry = Some(
                m.expiry
                    .map_or(now + CRYPTOR_EXPIRY, |e| e.min(now + CRYPTOR_EXPIRY)),
            );
        }
        self.managers.push_back(Manager::new(base_secret, now));
    }

    /// Şifresiz kareleri süresiz kabul et (DAVE'siz çağrı ya da sürüm 0'a geçiş).
    pub fn enable_passthrough(&mut self) {
        self.passthrough_forever = true;
    }

    /// Şifresiz kareleri en fazla `grace` sonrasına kadar kabul et (uzatılamaz).
    pub fn disable_passthrough(&mut self, now: Instant, grace: Duration) {
        let limit = now + grace;
        self.passthrough_until = Some(if self.passthrough_forever {
            limit
        } else {
            self.passthrough_until.map_or(now, |u| u.min(limit))
        });
        self.passthrough_forever = false;
    }

    fn passthrough(&self, now: Instant) -> bool {
        self.passthrough_forever || self.passthrough_until.is_some_and(|u| now < u)
    }

    /// Kareyi çözer; çözülemezse `None` (kare atılır).
    pub fn decrypt(&mut self, data: &[u8], now: Instant) -> Option<Vec<u8>> {
        if data == OPUS_SILENCE {
            return Some(data.to_vec());
        }
        self.managers.retain(|m| !m.expired(now));
        let Some(p) = frame::parse(data) else {
            return self.passthrough(now).then(|| data.to_vec());
        };
        self.managers
            .iter_mut()
            .rev()
            .find_map(|m| m.try_decrypt(&p, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: [u8; 16] = [1; 16];
    const B: [u8; 16] = [2; 16];

    #[test]
    fn wrapped_generation_examples() {
        assert_eq!(wrapped_generation(0, 0), 0);
        assert_eq!(wrapped_generation(0, 5), 5);
        assert_eq!(wrapped_generation(255, 0), 256);
        assert_eq!(wrapped_generation(256, 1), 257);
        assert_eq!(wrapped_generation(300, 10), 512 + 10);
    }

    fn pair() -> (Encryptor, Decryptor, Instant) {
        let now = Instant::now();
        let mut e = Encryptor::default();
        e.set_ratchet(&A);
        let mut d = Decryptor::default();
        d.add_ratchet(&A, now);
        (e, d, now)
    }

    #[test]
    fn roundtrip_and_replay_rejected() {
        let (mut e, mut d, now) = pair();
        let mut frames = Vec::new();
        for i in 0..3u8 {
            let mut out = Vec::new();
            assert!(e.encrypt(&[i; 20], &mut out));
            frames.push(out);
        }
        assert_eq!(frames[0][frames[0].len() - 4], 1, "ilk nonce 1");
        // Sırasız: 2 önce, sonra 0 ve 1 (eksik listesinden) kabul.
        assert_eq!(d.decrypt(&frames[2], now), Some(vec![2; 20]));
        assert_eq!(d.decrypt(&frames[0], now), Some(vec![0; 20]));
        assert_eq!(d.decrypt(&frames[1], now), Some(vec![1; 20]));
        // Hepsi tekrar: reddedilir.
        for f in &frames {
            assert_eq!(d.decrypt(f, now), None);
        }
    }

    #[test]
    fn silence_and_passthrough() {
        let (mut e, mut d, now) = pair();
        let mut out = Vec::new();
        assert!(e.encrypt(&OPUS_SILENCE, &mut out));
        assert_eq!(out, OPUS_SILENCE);
        assert_eq!(d.decrypt(&OPUS_SILENCE, now), Some(OPUS_SILENCE.to_vec()));
        // Şifresiz kare: izin yoksa atılır.
        assert_eq!(d.decrypt(b"ham opus", now), None);
        d.enable_passthrough();
        assert_eq!(d.decrypt(b"ham opus", now), Some(b"ham opus".to_vec()));
        d.disable_passthrough(now, Duration::from_secs(10));
        assert!(
            d.decrypt(b"ham opus", now + Duration::from_secs(9))
                .is_some()
        );
        assert!(
            d.decrypt(b"ham opus", now + Duration::from_secs(11))
                .is_none()
        );
        // Uzatılamaz.
        d.disable_passthrough(now + Duration::from_secs(5), Duration::from_secs(60));
        assert!(
            d.decrypt(b"ham opus", now + Duration::from_secs(12))
                .is_none()
        );
        // Şifreleyici sürüm 0'da şifrelemez.
        e.set_passthrough();
        assert!(e.encrypt(b"x", &mut out));
        assert_eq!(out, b"x");
    }

    #[test]
    fn epoch_transition_keeps_old_keys_briefly() {
        let (mut e_old, mut d, now) = pair();
        let mut e_new = Encryptor::default();
        e_new.set_ratchet(&B);
        let mut old = Vec::new();
        let mut new = Vec::new();
        e_old.encrypt(b"eski", &mut old);
        e_new.encrypt(b"yeni", &mut new);
        assert_eq!(d.decrypt(&new, now), None, "yeni anahtar henüz yok");
        d.add_ratchet(&B, now);
        assert_eq!(d.decrypt(&new, now).as_deref(), Some(b"yeni".as_slice()));
        assert_eq!(
            d.decrypt(&old, now + Duration::from_secs(5)).as_deref(),
            Some(b"eski".as_slice())
        );
        let mut old2 = Vec::new();
        e_old.encrypt(b"eski2", &mut old2);
        assert_eq!(
            d.decrypt(&old2, now + Duration::from_secs(11)),
            None,
            "süresi doldu"
        );
        // Aynı sır tekrar eklenirse yok sayılır.
        d.add_ratchet(&B, now);
        assert_eq!(d.managers.len(), 1);
    }

    #[test]
    fn no_key_means_no_send() {
        let mut e = Encryptor::default();
        let mut out = Vec::new();
        assert!(!e.ready());
        assert!(!e.encrypt(b"x", &mut out));
    }

    #[test]
    fn generation_advance_is_rate_limited() {
        // Nesil 1'e geçen kare: yeni yönetici için çok erken (sınır ~27 saat/nesil).
        let now = Instant::now();
        let mut e = Encryptor::default();
        e.set_ratchet(&A);
        e.nonce = 0x00FF_FFFF; // sonraki kare nesil 1
        let mut out = Vec::new();
        assert!(e.encrypt(b"n1", &mut out));
        let mut d = Decryptor::default();
        d.add_ratchet(&A, now);
        assert_eq!(d.decrypt(&out, now), None);
        assert_eq!(
            d.decrypt(&out, now + Duration::from_secs(30 * 3600))
                .as_deref(),
            Some(b"n1".as_slice())
        );
    }
}
