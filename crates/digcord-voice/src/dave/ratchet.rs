//! DAVE anahtar zinciri ("hash ratchet"): MLS gizli ağaç zincirinin (RFC 9420 §9.1) çıkarma
//! adımı olmayan hali. Her üyenin her epoch'taki 16 baytlık taban sırrından nesil başına
//! AES-128 anahtarı türetir.
//!
//! `hkdf::Hkdf::from_prk` 32 bayttan kısa PRK'yı reddettiği için HKDF-Expand elle (HMAC-SHA256).

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const KEY_LEN: usize = 16;
/// Tek adımda en fazla bu kadar nesil ileri gidilir (saldırgan zinciri ilerletmesin).
const MAX_FORWARD: u32 = 1 << 16;

/// RFC 9420 ExpandWithLabel (SHA-256), çıktı ≤ 32 bayt (tek HKDF bloğu).
fn expand_with_label(secret: &[u8], label: &str, context: &[u8], len: usize) -> [u8; 32] {
    debug_assert!(len <= 32);
    const PREFIX: &str = "MLS 1.0 ";
    let full_label_len = PREFIX.len() + label.len();
    // Uzunluklar < 64: MLS değişken uzunluk öneki tek bayt.
    let mut info = Vec::with_capacity(4 + full_label_len + context.len() + 1);
    info.extend_from_slice(&u16::try_from(len).unwrap_or(32).to_be_bytes());
    info.push(u8::try_from(full_label_len).unwrap_or(0));
    info.extend_from_slice(PREFIX.as_bytes());
    info.extend_from_slice(label.as_bytes());
    info.push(u8::try_from(context.len()).unwrap_or(0));
    info.extend_from_slice(context);
    info.push(1); // HKDF-Expand sayaç baytı T(1)
    // HMAC her uzunlukta anahtarı kabul eder; hata imkânsız.
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        return [0; 32];
    };
    mac.update(&info);
    mac.finalize().into_bytes().into()
}

fn derive_tree_secret(secret: &[u8], label: &str, generation: u32, len: usize) -> [u8; 32] {
    expand_with_label(secret, label, &generation.to_be_bytes(), len)
}

pub struct HashRatchet {
    next_secret: Vec<u8>,
    next_generation: u32,
    keys: BTreeMap<u32, [u8; KEY_LEN]>,
}

impl HashRatchet {
    pub fn new(base_secret: &[u8]) -> Self {
        Self {
            next_secret: base_secret.to_vec(),
            next_generation: 0,
            keys: BTreeMap::new(),
        }
    }

    /// `generation` nesli anahtarı; silinmiş ya da çok ileride ise `None`.
    pub fn get(&mut self, generation: u32) -> Option<[u8; KEY_LEN]> {
        if let Some(k) = self.keys.get(&generation) {
            return Some(*k);
        }
        if generation < self.next_generation || generation - self.next_generation > MAX_FORWARD {
            return None;
        }
        while self.next_generation <= generation {
            let g = self.next_generation;
            let block = derive_tree_secret(&self.next_secret, "key", g, KEY_LEN);
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&block[..KEY_LEN]);
            self.keys.insert(g, key);
            self.next_secret = derive_tree_secret(&self.next_secret, "secret", g, 32).to_vec();
            self.next_generation = g.checked_add(1)?;
        }
        self.keys.get(&generation).copied()
    }

    /// Artık gerekmeyen nesli unutur (bir daha türetilemez).
    pub fn erase(&mut self, generation: u32) {
        self.keys.remove(&generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    const BASE: &str = "cedd61b1b8a1ca69046554282cf70b7b";

    #[test]
    fn known_vector_generation_0() {
        // davey (MIT) test vektörü; bağımsız Python türetimiyle de tutuyor.
        let mut r = HashRatchet::new(&hex(BASE));
        assert_eq!(
            r.get(0).expect("anahtar").to_vec(),
            hex("7530f9a9945e2d2e06d0651f7b2a864b")
        );
        let nonce = derive_tree_secret(&hex(BASE), "nonce", 0, 12);
        assert_eq!(nonce[..12].to_vec(), hex("301e5f4b74090f985e726bb2"));
    }

    #[test]
    fn generation_1_and_next_secret() {
        // Kendi hesaplanan (spesifikasyondaki Python referansı) değerler.
        let next = derive_tree_secret(&hex(BASE), "secret", 0, 32);
        assert_eq!(
            next.to_vec(),
            hex("94c10756040f20134afedc3cd3b6985051c3335128e127d770f2fada84a2c50c")
        );
        let mut r = HashRatchet::new(&hex(BASE));
        assert_eq!(
            r.get(1).expect("anahtar").to_vec(),
            hex("cddd06e0f3437596bacb9c06a0329c49")
        );
        // Atlanan nesil de önbellekte.
        assert_eq!(
            r.get(0).expect("anahtar").to_vec(),
            hex("7530f9a9945e2d2e06d0651f7b2a864b")
        );
    }

    #[test]
    fn erased_and_far_generations_are_refused() {
        let mut r = HashRatchet::new(&hex(BASE));
        assert!(r.get(3).is_some());
        r.erase(1);
        assert!(r.get(1).is_none(), "silinen nesil geri gelmez");
        assert!(r.get(2).is_some());
        assert!(r.get(4 + MAX_FORWARD + 1).is_none());
    }
}
