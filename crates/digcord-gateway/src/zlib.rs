//! Gateway `compress=zlib-stream` açıcısı.
//!
//! Bağlantı boyunca tek bir zlib akışı vardır; her gateway mesajı `00 00 ff ff`
//! (sync flush) son ekiyle biter. Bir mesaj birden fazla WebSocket çerçevesine
//! bölünebilir, bu yüzden son ek gelene kadar baytlar biriktirilir.
//! Her yeni bağlantıda yeni bir [`ZlibStream`] oluşturulmalı.

use flate2::{Decompress, FlushDecompress, Status};

const SUFFIX: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
/// Çıkış tamponu dolunca en az bu kadar büyütülür.
const MIN_GROW: usize = 32 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ZlibError {
    #[error("zlib akışı bozuk: {0}")]
    Corrupt(#[from] flate2::DecompressError),
    #[error("zlib akışı ilerlemiyor")]
    Stalled,
}

/// Kalıcı zlib-stream açıcısı. Tamponlar mesajlar arasında yeniden kullanılır.
pub struct ZlibStream {
    inflate: Decompress,
    pending: Vec<u8>,
    out: Vec<u8>,
}

impl Default for ZlibStream {
    fn default() -> Self {
        Self::new()
    }
}

impl ZlibStream {
    pub fn new() -> Self {
        Self {
            inflate: Decompress::new(true),
            pending: Vec::new(),
            out: Vec::new(),
        }
    }

    /// Bir binary çerçeve ekler. Mesaj tamamlandıysa açılmış baytları döndürür,
    /// tamamlanmadıysa `None`. Dönen dilim bir sonraki çağrıya kadar geçerlidir.
    pub fn push(&mut self, frame: &[u8]) -> Result<Option<&[u8]>, ZlibError> {
        self.pending.extend_from_slice(frame);
        if !self.pending.ends_with(&SUFFIX) {
            return Ok(None);
        }

        self.out.clear();
        let mut input = &self.pending[..];
        loop {
            let spare = self.out.capacity() - self.out.len();
            if spare < 1024 {
                self.out.reserve(MIN_GROW.max(input.len() * 4));
            }
            let in_before = self.inflate.total_in();
            let out_before = self.out.len();
            let status =
                self.inflate
                    .decompress_vec(input, &mut self.out, FlushDecompress::Sync)?;
            let consumed = (self.inflate.total_in() - in_before) as usize;
            input = &input[consumed..];

            let produced = self.out.len() - out_before;
            let out_full = self.out.len() == self.out.capacity();
            if status == Status::StreamEnd || (input.is_empty() && !out_full) {
                break;
            }
            if consumed == 0 && produced == 0 && !out_full {
                return Err(ZlibError::Stalled);
            }
        }
        self.pending.clear();
        Ok(Some(&self.out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    /// Sunucu gibi davranan sıkıştırıcı: tek akış, her mesaj sync flush ile biter.
    struct Server(Compress);

    impl Server {
        fn new() -> Self {
            Self(Compress::new(Compression::default(), true))
        }

        fn send(&mut self, msg: &[u8]) -> Vec<u8> {
            let mut out = Vec::with_capacity(msg.len() + 64);
            let mut input = msg;
            loop {
                out.reserve(MIN_GROW);
                let before = self.0.total_in();
                self.0
                    .compress_vec(input, &mut out, FlushCompress::Sync)
                    .expect("sıkıştırma");
                input = &input[(self.0.total_in() - before) as usize..];
                if input.is_empty() && out.ends_with(&SUFFIX) {
                    return out;
                }
            }
        }
    }

    #[test]
    fn several_messages_on_one_stream() {
        let mut server = Server::new();
        let mut client = ZlibStream::new();
        for i in 0..20 {
            let msg = format!(
                r#"{{"op":0,"s":{i},"t":"MESSAGE_CREATE","d":{{"content":"merhaba {i}"}}}}"#
            );
            let frame = server.send(msg.as_bytes());
            let out = client.push(&frame).expect("açma").expect("tam mesaj");
            assert_eq!(out, msg.as_bytes());
        }
    }

    #[test]
    fn message_split_across_frames() {
        let mut server = Server::new();
        let mut client = ZlibStream::new();
        let msg = br#"{"op":10,"d":{"heartbeat_interval":41250}}"#;
        let frame = server.send(msg);
        let (a, b) = frame.split_at(frame.len() / 2);
        assert!(client.push(a).expect("açma").is_none());
        assert_eq!(client.push(b).expect("açma").expect("tam mesaj"), msg);
    }

    #[test]
    fn large_message_grows_buffer() {
        let mut server = Server::new();
        let mut client = ZlibStream::new();
        // Kötü sıkışan, ~2 MB'lık sözde rastgele veri.
        let mut x: u32 = 1;
        let msg: Vec<u8> = (0..2_000_000)
            .map(|_| {
                x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                b'a' + (x >> 16) as u8 % 26
            })
            .collect();
        let frame = server.send(&msg);
        assert_eq!(
            client.push(&frame).expect("açma").expect("tam mesaj"),
            &msg[..]
        );
        // Arkasından küçük mesaj aynı akışta çalışmaya devam eder.
        let frame = server.send(b"{}");
        assert_eq!(
            client.push(&frame).expect("açma").expect("tam mesaj"),
            b"{}"
        );
    }

    #[test]
    fn corrupt_stream_is_error() {
        let mut client = ZlibStream::new();
        let garbage = [0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0xff, 0xff];
        assert!(client.push(&garbage).is_err());
    }
}
