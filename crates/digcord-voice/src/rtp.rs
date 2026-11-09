//! RTP başlığı (RFC 3550) — Discord'un kullandığı alt küme.

/// Sabit başlık boyutu (CSRC ve uzantı hariç).
pub const HEADER_LEN: usize = 12;
/// Opus yük türü.
pub const OPUS_PAYLOAD_TYPE: u8 = 0x78;
/// Sürüm 2, dolgu yok, uzantı yok, CSRC yok.
const FIRST_BYTE: u8 = 0x80;

/// Giden paketin başlığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
}

impl Header {
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(FIRST_BYTE);
        out.push(OPUS_PAYLOAD_TYPE);
        out.extend_from_slice(&self.sequence.to_be_bytes());
        out.extend_from_slice(&self.timestamp.to_be_bytes());
        out.extend_from_slice(&self.ssrc.to_be_bytes());
    }
}

/// Gelen bir RTP paketinin başlık bilgisi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parsed {
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload_type: u8,
    /// "rtpsize" kiplerinde şifrelenmeden kalan (AAD) kısım: sabit başlık + CSRC'ler +
    /// (varsa) 4 baytlık uzantı başlığı.
    pub aad_len: usize,
    /// Şifreli kısmın başındaki uzantı verisinin uzunluğu (çözdükten sonra atılır).
    pub extension_len: usize,
}

/// RTCP paketi mi (yük türü 72..=76, işaret biti hariç; RFC 5761).
pub fn is_rtcp(packet: &[u8]) -> bool {
    packet.len() >= 2 && (72..=76).contains(&(packet[1] & 0x7F))
}

/// Başlığı ayrıştırır; sürüm 2 değilse ya da paket kısaysa `None`.
pub fn parse(packet: &[u8]) -> Option<Parsed> {
    if packet.len() < HEADER_LEN || packet[0] >> 6 != 2 {
        return None;
    }
    let csrc = usize::from(packet[0] & 0x0F);
    let has_extension = packet[0] & 0x10 != 0;
    let mut aad_len = HEADER_LEN + csrc * 4;
    let mut extension_len = 0;
    if has_extension {
        let words = packet.get(aad_len + 2..aad_len + 4)?;
        extension_len = usize::from(u16::from_be_bytes([words[0], words[1]])) * 4;
        aad_len += 4;
    }
    if packet.len() < aad_len {
        return None;
    }
    Some(Parsed {
        sequence: u16::from_be_bytes([packet[2], packet[3]]),
        timestamp: u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]),
        ssrc: u32::from_be_bytes([packet[8], packet[9], packet[10], packet[11]]),
        payload_type: packet[1] & 0x7F,
        aad_len,
        extension_len,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_then_parse() {
        let h = Header {
            sequence: 0x0102,
            timestamp: 0x0304_0506,
            ssrc: 0x0708_090A,
        };
        let mut buf = Vec::new();
        h.write(&mut buf);
        assert_eq!(buf, [0x80, 0x78, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        let p = parse(&buf).expect("başlık");
        assert_eq!(
            (p.sequence, p.timestamp, p.ssrc),
            (0x0102, 0x0304_0506, 0x0708_090A)
        );
        assert_eq!((p.payload_type, p.aad_len, p.extension_len), (0x78, 12, 0));
    }

    #[test]
    fn extension_and_csrc() {
        // X biti + 1 CSRC; uzantı başlığı: profil 0xBEDE, 2 kelime.
        let mut buf = vec![0x91, 0x78, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3];
        buf.extend_from_slice(&[0, 0, 0, 9]); // CSRC
        buf.extend_from_slice(&[0xBE, 0xDE, 0, 2]);
        buf.extend_from_slice(&[0; 8]);
        let p = parse(&buf).expect("başlık");
        assert_eq!((p.aad_len, p.extension_len), (20, 8));
    }

    #[test]
    fn rejects_short_or_wrong_version() {
        assert!(parse(&[0x80; 11]).is_none());
        assert!(parse(&[0x40; 12]).is_none());
        // Uzantı biti var ama uzantı başlığı yok.
        assert!(parse(&[0x90, 0x78, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_none());
    }

    #[test]
    fn rtcp_detection() {
        assert!(is_rtcp(&[0x80, 0xC8]));
        assert!(is_rtcp(&[0x81, 0xC9]));
        assert!(!is_rtcp(&[0x80, 0x78]));
    }
}
