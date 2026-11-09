//! DAVE kare şifrelemesi (ses: Opus karesinin tamamı şifreli).
//!
//! Çerçeve: şifreli metin | etiket (8) | ULEB128(nonce) | [ULEB128(ofset) ULEB128(boyut)]* |
//! ek boyut (u8; etiketten sona kadar) | 0xFA 0xFA.
//! AES-128-GCM, 12 baytlık nonce = 8 sıfır + nonce (u32, küçük endian); etiket 16 baytlık
//! GCM etiketinin ilk 8 baytı. AAD: şifrelenmeyen aralıkların birleşimi (Opus'ta boş).

use aes_gcm::Aes128Gcm;
use aes_gcm::aead::{AeadInOut, KeyInit};
use subtle::ConstantTimeEq;

pub const TAG_LEN: usize = 8;
pub const MAGIC: [u8; 2] = [0xFA, 0xFA];
/// Opus sessizlik karesi: DAVE ile şifrelenmez, aynen geçer.
pub const OPUS_SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];
/// En küçük ek bölüm: etiket + 1 baytlık nonce + boyut + sihirli işaret.
const MIN_SUPPLEMENTAL: usize = TAG_LEN + 1 + 1 + 2;

pub fn write_uleb128(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `(değer, okunan bayt)`; taşma ya da eksik veri → `None`.
pub fn read_uleb128(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for (i, b) in bytes.iter().enumerate().take(10) {
        let part = u64::from(b & 0x7F);
        let shift = 7 * i as u32;
        if shift == 63 && part > 1 {
            return None;
        }
        v |= part << shift;
        if b & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

fn gcm_nonce(nonce: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..].copy_from_slice(&nonce.to_le_bytes());
    n
}

/// Opus karesini şifreleyip DAVE çerçevesini `out`'a yazar (`out` temizlenir).
pub fn encrypt(key: &[u8; 16], nonce: u32, plaintext: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.extend_from_slice(plaintext);
    let cipher = Aes128Gcm::new(key.into());
    let Ok(tag) =
        cipher.encrypt_inout_detached(&gcm_nonce(nonce).into(), &[], out.as_mut_slice().into())
    else {
        // Sadece aşırı uzun girdide; ses karesi bunun çok altında.
        out.clear();
        return;
    };
    let start = out.len();
    out.extend_from_slice(&tag[..TAG_LEN]);
    write_uleb128(u64::from(nonce), out);
    let size = out.len() - start + 1 + MAGIC.len();
    out.push(u8::try_from(size).unwrap_or(u8::MAX));
    out.extend_from_slice(&MAGIC);
}

/// Ayrıştırılmış DAVE çerçevesi.
#[derive(Debug, PartialEq, Eq)]
pub struct Protected<'a> {
    frame: &'a [u8],
    /// Çerçevenin şifreli/aralıklı kısmının uzunluğu (ek bölümden önce).
    body_len: usize,
    tag: [u8; TAG_LEN],
    pub nonce: u32,
    /// Şifrelenmeden kalan aralıklar (ofset, boyut).
    ranges: Vec<(usize, usize)>,
}

/// DAVE çerçevesi mi; değilse `None` (geçiş izni varsa ham kare olarak kullanılabilir).
pub fn parse(frame: &[u8]) -> Option<Protected<'_>> {
    let len = frame.len();
    if len < MIN_SUPPLEMENTAL || frame[len - 2..] != MAGIC {
        return None;
    }
    let size = usize::from(frame[len - 3]);
    if size < MIN_SUPPLEMENTAL || size > len {
        return None;
    }
    let body_len = len - size;
    let sup = &frame[body_len..len - 3];
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&sup[..TAG_LEN]);
    let (nonce, used) = read_uleb128(&sup[TAG_LEN..])?;
    let nonce = u32::try_from(nonce).ok()?;
    let mut rest = &sup[TAG_LEN + used..];
    let mut ranges = Vec::new();
    let mut prev_end = 0usize;
    while !rest.is_empty() {
        let (offset, a) = read_uleb128(rest)?;
        let (size, b) = read_uleb128(rest.get(a..)?)?;
        rest = &rest[a + b..];
        let offset = usize::try_from(offset).ok()?;
        let end = offset.checked_add(usize::try_from(size).ok()?)?;
        if offset < prev_end || end > body_len {
            return None;
        }
        ranges.push((offset, end - offset));
        prev_end = end;
    }
    Some(Protected {
        frame,
        body_len,
        tag,
        nonce,
        ranges,
    })
}

impl Protected<'_> {
    /// Doğrular ve çözer; etiket tutmazsa `None`.
    pub fn decrypt(&self, key: &[u8; 16]) -> Option<Vec<u8>> {
        let body = &self.frame[..self.body_len];
        let mut aad = Vec::new();
        let mut ciphertext = Vec::with_capacity(body.len());
        let mut pos = 0;
        for &(off, size) in &self.ranges {
            ciphertext.extend_from_slice(&body[pos..off]);
            aad.extend_from_slice(&body[off..off + size]);
            pos = off + size;
        }
        ciphertext.extend_from_slice(&body[pos..]);

        // GCM şifre çözmesi CTR'dir: aynı anahtar/nonce ile "şifrelemek" düz metni verir.
        // Sonra düz metni yeniden şifreleyip asıl etiketi hesaplar, ilk 8 baytını sabit
        // sürede karşılaştırırız. (Standart crate 8 baytlık etikete izin vermiyor.)
        let cipher = Aes128Gcm::new(key.into());
        let nonce = gcm_nonce(self.nonce).into();
        let mut plain = ciphertext.clone();
        cipher
            .encrypt_inout_detached(&nonce, &aad, plain.as_mut_slice().into())
            .ok()?;
        let mut check = plain.clone();
        let tag = cipher
            .encrypt_inout_detached(&nonce, &aad, check.as_mut_slice().into())
            .ok()?;
        if !bool::from(tag[..TAG_LEN].ct_eq(&self.tag)) {
            return None;
        }
        // Düz metni aralıkların arasına geri yerleştir.
        if self.ranges.is_empty() {
            return Some(plain);
        }
        let mut out = Vec::with_capacity(body.len());
        let mut p = 0;
        let mut pos = 0;
        for &(off, size) in &self.ranges {
            let n = off - pos;
            out.extend_from_slice(&plain[p..p + n]);
            p += n;
            out.extend_from_slice(&body[off..off + size]);
            pos = off + size;
        }
        out.extend_from_slice(&plain[p..]);
        Some(out)
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

    fn key(s: &str) -> [u8; 16] {
        hex(s).try_into().expect("16 bayt")
    }

    const PLAIN: &str = "0dc5aedd5bdc3f20be5697e54dd1f437b896a36f858c6f20bbd69e2a493ca170c4f0c1b9acd49d324b92afa788d09b12b29115a2feb3552b60fff983234a6c9608af3933683efc6b0f5579a9";
    const GEN0: &str = "7530f9a9945e2d2e06d0651f7b2a864b";
    const GEN1: &str = "cddd06e0f3437596bacb9c06a0329c49";

    #[test]
    fn uleb128_roundtrip_and_examples() {
        for (v, enc) in [
            (1u64, "01"),
            (300, "ac02"),
            (0x0100_0000, "80808008"),
            (0, "00"),
        ] {
            let mut out = Vec::new();
            write_uleb128(v, &mut out);
            assert_eq!(out, hex(enc));
            assert_eq!(read_uleb128(&out), Some((v, out.len())));
        }
        assert_eq!(read_uleb128(&[0x80]), None, "eksik");
        assert_eq!(read_uleb128(&[0xFF; 11]), None, "taşma");
    }

    /// Spesifikasyondaki (Python `cryptography` ile bağımsız hesaplanmış) vektörler.
    #[test]
    fn matches_cross_check_vectors() {
        let cases = [
            (
                GEN0,
                1u32,
                "dce00536145ccf3d15a0da9a282bb6e2f7c385025226be687d281cf21f38f223ad3b46ac44a21b3c9ea47b189a3e25b11e12e5eca8d112be4e7942a037c3e58bd6861f74303383a6fe0ef2400773a2c4400633a9010cfafa",
            ),
            (
                GEN1,
                0x0100_0000,
                "3b1ae251114b6ce6fcad4311f6227e7d7c2793c4c831657dab93f3cd8fb1039a02cf861a4e2afd4c3af6eecf08ddc26db176e4f28f9ecf89e3625adbf9951d07d097fc666fe87fd22b93f396dc03188e66d0b58a808080080ffafa",
            ),
            (
                GEN0,
                300,
                "36e78656aca685997db4d30a0e026dc93fa531d9d3b1a70cdf1bda41313decd9a31063fa555d90c2dafc67549d2f49126c9dc5e1c15b3be53e7c2d273c721144172712764a6e20b5c6842008b667b82a0059b24cac020dfafa",
            ),
        ];
        for (k, nonce, expected) in cases {
            let mut out = Vec::new();
            encrypt(&key(k), nonce, &hex(PLAIN), &mut out);
            assert_eq!(out, hex(expected), "nonce {nonce}");
            let p = parse(&out).expect("çerçeve");
            assert_eq!(p.nonce, nonce);
            assert_eq!(p.decrypt(&key(k)), Some(hex(PLAIN)));
        }
    }

    #[test]
    fn tampering_or_wrong_key_fails() {
        let mut out = Vec::new();
        encrypt(&key(GEN0), 7, b"opus", &mut out);
        assert!(parse(&out).expect("çerçeve").decrypt(&key(GEN1)).is_none());
        for i in 0..out.len() - 3 {
            let mut bad = out.clone();
            bad[i] ^= 0x01;
            let ok = parse(&bad).and_then(|p| p.decrypt(&key(GEN0)));
            assert!(
                ok.is_none() || ok.as_deref() != Some(b"opus".as_slice()),
                "bayt {i}"
            );
        }
    }

    #[test]
    fn non_protocol_frames() {
        assert!(parse(&OPUS_SILENCE).is_none());
        assert!(parse(b"sadece opus verisi").is_none());
        // Boyut baytı çerçeveden büyük.
        let mut f = vec![0u8; 12];
        f[9] = 200;
        f[10..].copy_from_slice(&MAGIC);
        assert!(parse(&f).is_none());
    }

    #[test]
    fn unencrypted_ranges_are_aad_and_restored() {
        // Elle: ilk 2 bayt açık (AAD), kalan 3 bayt şifreli.
        let k = key(GEN0);
        let body = *b"ABcde";
        let cipher = Aes128Gcm::new((&k).into());
        let mut ct = body[2..].to_vec();
        let tag = cipher
            .encrypt_inout_detached(&gcm_nonce(5).into(), &body[..2], ct.as_mut_slice().into())
            .expect("şifrele");
        let mut frame = body[..2].to_vec();
        frame.extend_from_slice(&ct);
        let start = frame.len();
        frame.extend_from_slice(&tag[..8]);
        frame.extend_from_slice(&[5, 0, 2]); // nonce, aralık (0, 2)
        let size = frame.len() - start + 3;
        frame.push(size as u8);
        frame.extend_from_slice(&MAGIC);
        let p = parse(&frame).expect("çerçeve");
        assert_eq!(p.decrypt(&k).as_deref(), Some(b"ABcde".as_slice()));
    }
}
