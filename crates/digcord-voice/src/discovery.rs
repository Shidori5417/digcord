//! UDP IP keşfi: sunucuya kendi SSRC'mizi gönderip dış IP ve portumuzu öğreniriz.
//!
//! Paket (74 bayt, büyük endian): tür (u16; 1 istek, 2 yanıt), uzunluk (u16 = 70),
//! SSRC (u32), adres (64 bayt, NUL ile biten ASCII), port (u16).

pub const PACKET_LEN: usize = 74;
const BODY_LEN: u16 = 70;
const REQUEST: u16 = 1;
const RESPONSE: u16 = 2;
const ADDRESS_LEN: usize = 64;

pub fn request(ssrc: u32) -> [u8; PACKET_LEN] {
    let mut p = [0u8; PACKET_LEN];
    p[0..2].copy_from_slice(&REQUEST.to_be_bytes());
    p[2..4].copy_from_slice(&BODY_LEN.to_be_bytes());
    p[4..8].copy_from_slice(&ssrc.to_be_bytes());
    p
}

/// Yanıttan (IP, port). Bozuk ya da başka SSRC'ye aitse `None`.
pub fn parse_response(packet: &[u8], ssrc: u32) -> Option<(String, u16)> {
    if packet.len() < PACKET_LEN
        || packet[0..2] != RESPONSE.to_be_bytes()
        || packet[4..8] != ssrc.to_be_bytes()
    {
        return None;
    }
    let addr = &packet[8..8 + ADDRESS_LEN];
    let end = addr.iter().position(|b| *b == 0).unwrap_or(ADDRESS_LEN);
    let ip = std::str::from_utf8(&addr[..end]).ok()?;
    if ip.is_empty() || !ip.is_ascii() {
        return None;
    }
    let port = u16::from_be_bytes([packet[72], packet[73]]);
    Some((ip.to_owned(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_layout() {
        let p = request(0x0102_0304);
        assert_eq!(&p[..8], &[0, 1, 0, 70, 1, 2, 3, 4]);
        assert!(p[8..].iter().all(|b| *b == 0));
    }

    #[test]
    fn parses_response() {
        let mut p = [0u8; PACKET_LEN];
        p[0..2].copy_from_slice(&[0, 2]);
        p[2..4].copy_from_slice(&[0, 70]);
        p[4..8].copy_from_slice(&7u32.to_be_bytes());
        p[8..8 + 11].copy_from_slice(b"203.0.113.5");
        p[72..74].copy_from_slice(&50_000u16.to_be_bytes());
        assert_eq!(parse_response(&p, 7), Some(("203.0.113.5".into(), 50_000)));
        assert_eq!(parse_response(&p, 8), None, "başka SSRC");
        assert_eq!(parse_response(&p[..70], 7), None, "kısa");
        p[1] = 1;
        assert_eq!(parse_response(&p, 7), None, "istek türü");
    }
}
