//! DAVE durum makinesi: ses gateway'inden gelen DAVE mesajlarını MLS oturumuna uygular,
//! geçişleri (transition) yönetir ve şifreleyici/çözücülere anahtarları kurar.
//! Ağ yok: gelen mesajı alır, gönderilecek mesajları döner (PROTOCOL_NOTES §7, spesifikasyon §3.2).

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use super::cryptor::{CRYPTOR_EXPIRY, Decryptor, Encryptor};
use super::session::{Session, SessionError};
use crate::protocol;

/// Sunucudan gelen ikili DAVE mesajları.
pub mod binary_op {
    pub const EXTERNAL_SENDER: u8 = 25;
    pub const KEY_PACKAGE: u8 = 26;
    pub const PROPOSALS: u8 = 27;
    pub const COMMIT_WELCOME: u8 = 28;
    pub const ANNOUNCE_COMMIT: u8 = 29;
    pub const WELCOME: u8 = 30;
}

/// Ses gateway'ine gönderilecek mesaj.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    Json(String),
    Binary(Vec<u8>),
}

/// Sunucudan gelen ikili çerçeve: `[seq u16][opcode u8][yük]`.
pub fn split_server_binary(frame: &[u8]) -> Option<(u16, u8, &[u8])> {
    if frame.len() < 3 {
        return None;
    }
    Some((
        u16::from_be_bytes([frame[0], frame[1]]),
        frame[2],
        &frame[3..],
    ))
}

fn binary(op: u8, payload: &[u8]) -> Outgoing {
    let mut out = Vec::with_capacity(payload.len() + 1);
    out.push(op);
    out.extend_from_slice(payload);
    Outgoing::Binary(out)
}

fn split_transition(payload: &[u8]) -> Option<(u16, &[u8])> {
    (payload.len() >= 2).then(|| (u16::from_be_bytes([payload[0], payload[1]]), &payload[2..]))
}

pub struct Dave {
    session: Session,
    /// Şu anki (anlaşılan) DAVE sürümü; 0 = uçtan uca şifreleme yok.
    version: u16,
    /// Ses kanalında olduğunu sunucunun bildirdiği kullanıcılar (+ ben).
    recognized: HashSet<u64>,
    /// transition_id → protokol sürümü.
    pending: HashMap<u16, u16>,
    encryptor: Encryptor,
    decryptors: HashMap<u64, Decryptor>,
}

impl Dave {
    pub fn new(user_id: u64, channel_id: u64) -> Result<Self, SessionError> {
        Ok(Self {
            session: Session::new(user_id, channel_id)?,
            version: 0,
            recognized: HashSet::from([user_id]),
            pending: HashMap::new(),
            encryptor: Encryptor::default(),
            decryptors: HashMap::new(),
        })
    }

    pub fn version(&self) -> u16 {
        self.version
    }

    /// Kendi karemizi gönderilecek biçime çevirir (DAVE ya da şifresiz).
    pub fn encrypt(&mut self, opus: &[u8], out: &mut Vec<u8>) -> bool {
        self.encryptor.encrypt(opus, out)
    }

    /// `user_id`'nin karesini çözer.
    pub fn decrypt(&mut self, user_id: u64, frame: &[u8], now: Instant) -> Option<Vec<u8>> {
        let passthrough = self.passthrough_allowed();
        self.decryptors
            .entry(user_id)
            .or_insert_with(|| new_decryptor(passthrough))
            .decrypt(frame, now)
    }

    /// Yeni çözücüler şifresiz kareleri kabul etsin mi: sürüm 0 ya da 0'a geçiş bekliyor.
    fn passthrough_allowed(&self) -> bool {
        self.version == 0 || self.pending.values().any(|v| *v == 0)
    }

    pub fn clients_connect(&mut self, ids: &[u64]) {
        self.recognized.extend(ids);
    }

    pub fn client_disconnect(&mut self, id: u64) {
        if id != self.session.user_id() {
            self.recognized.remove(&id);
            self.decryptors.remove(&id);
        }
    }

    /// op 4 Session Description.
    pub fn session_description(&mut self, version: u16) -> Vec<Outgoing> {
        self.version = version;
        if version == 0 {
            self.encryptor.set_passthrough();
            for d in self.decryptors.values_mut() {
                d.enable_passthrough();
            }
            return Vec::new();
        }
        self.fresh_key_package()
    }

    fn fresh_key_package(&mut self) -> Vec<Outgoing> {
        match self.session.key_package() {
            Ok(kp) => vec![binary(binary_op::KEY_PACKAGE, &kp)],
            Err(e) => {
                tracing::warn!("DAVE anahtar paketi üretilemedi: {e}");
                Vec::new()
            }
        }
    }

    /// Geçersiz commit/welcome: sunucuya bildir, durumu sıfırla, yeni anahtar paketi gönder.
    fn recover(&mut self, transition_id: u16) -> Vec<Outgoing> {
        let mut out = vec![Outgoing::Json(protocol::invalid_commit_welcome(
            transition_id,
        ))];
        self.session.reset();
        out.extend(self.fresh_key_package());
        out
    }

    /// Sunucudan gelen ikili mesaj (seq çıkarılmış).
    pub fn binary(&mut self, op: u8, payload: &[u8], now: Instant) -> Vec<Outgoing> {
        match op {
            binary_op::EXTERNAL_SENDER => {
                if let Err(e) = self.session.set_external_sender(payload) {
                    tracing::warn!("DAVE dış gönderici geçersiz: {e}");
                }
                Vec::new()
            }
            binary_op::PROPOSALS => match self.session.process_proposals(payload, &self.recognized)
            {
                Ok(Some(commit)) => vec![binary(binary_op::COMMIT_WELCOME, &commit)],
                Ok(None) => Vec::new(),
                Err(e) => {
                    tracing::warn!("DAVE önerileri işlenemedi: {e}");
                    Vec::new()
                }
            },
            binary_op::ANNOUNCE_COMMIT => {
                let Some((tid, commit)) = split_transition(payload) else {
                    return Vec::new();
                };
                let before = self.session.epoch();
                match self.session.process_commit(commit) {
                    // Henüz grupta değilken yok sayılan commit: geçiş yok.
                    Ok(()) if before.is_none() && self.session.epoch().is_none() => Vec::new(),
                    Ok(()) => self.group_changed(tid, now),
                    Err(e) => {
                        tracing::warn!(transition = tid, "DAVE commit geçersiz: {e}");
                        self.recover(tid)
                    }
                }
            }
            binary_op::WELCOME => {
                let Some((tid, welcome)) = split_transition(payload) else {
                    return Vec::new();
                };
                match self.session.process_welcome(welcome, &self.recognized) {
                    Ok(()) => self.group_changed(tid, now),
                    Err(e) => {
                        tracing::warn!(transition = tid, "DAVE welcome geçersiz: {e}");
                        self.recover(tid)
                    }
                }
            }
            other => {
                tracing::debug!(op = other, "bilinmeyen DAVE ikili mesajı");
                Vec::new()
            }
        }
    }

    /// Yeni epoch kuruldu: diğer üyelerin anahtarlarını hemen hazırla (onlar geçişten önce
    /// yeni anahtarla gönderebilir); kendi şifreleyicimiz geçiş yürütülünce değişir.
    fn group_changed(&mut self, tid: u16, now: Instant) -> Vec<Outgoing> {
        let me = self.session.user_id();
        for member in self.session.members() {
            if member == me {
                continue;
            }
            match self.session.sender_secret(member) {
                Ok(secret) => {
                    let passthrough = self.passthrough_allowed();
                    self.decryptors
                        .entry(member)
                        .or_insert_with(|| new_decryptor(passthrough))
                        .add_ratchet(&secret, now);
                }
                Err(e) => tracing::warn!("DAVE gönderici anahtarı türetilemedi: {e}"),
            }
        }
        let version = self.version.max(1);
        self.pending.insert(tid, version);
        if tid == 0 {
            self.execute_transition(0, now);
            Vec::new()
        } else {
            vec![Outgoing::Json(protocol::transition_ready(tid))]
        }
    }

    /// op 21.
    pub fn prepare_transition(&mut self, tid: u16, version: u16, now: Instant) -> Vec<Outgoing> {
        self.pending.insert(tid, version);
        if version == 0 {
            for d in self.decryptors.values_mut() {
                d.enable_passthrough();
            }
        }
        if tid == 0 {
            self.execute_transition(0, now);
            Vec::new()
        } else {
            vec![Outgoing::Json(protocol::transition_ready(tid))]
        }
    }

    /// op 22.
    pub fn execute_transition(&mut self, tid: u16, now: Instant) {
        let Some(version) = self.pending.remove(&tid) else {
            return;
        };
        self.version = version;
        if version == 0 {
            self.session.reset();
            self.encryptor.set_passthrough();
            return;
        }
        match self.session.sender_secret(self.session.user_id()) {
            Ok(secret) => self.encryptor.set_ratchet(&secret),
            Err(e) => tracing::warn!("DAVE kendi anahtarımız türetilemedi: {e}"),
        }
        for d in self.decryptors.values_mut() {
            d.disable_passthrough(now, CRYPTOR_EXPIRY);
        }
    }

    /// op 24: epoch 1 → yeni grup (yeni anahtar paketi).
    pub fn prepare_epoch(&mut self, epoch: u64, version: u16) -> Vec<Outgoing> {
        if epoch != 1 {
            return Vec::new();
        }
        self.version = version;
        self.session.reset();
        self.fresh_key_package()
    }
}

fn new_decryptor(passthrough: bool) -> Decryptor {
    let mut d = Decryptor::default();
    if passthrough {
        d.enable_passthrough();
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_binary_framing() {
        assert_eq!(
            split_server_binary(&[0, 7, 25, 1, 2]),
            Some((7, 25, [1u8, 2].as_slice()))
        );
        assert_eq!(split_server_binary(&[0, 7]), None);
        assert_eq!(split_transition(&[0, 3, 9]), Some((3, [9u8].as_slice())));
        assert_eq!(split_transition(&[0]), None);
        assert_eq!(binary(26, &[1, 2]), Outgoing::Binary(vec![26, 1, 2]));
    }

    #[test]
    fn version_zero_is_passthrough() {
        let now = Instant::now();
        let mut d = Dave::new(1, 10).expect("oturum");
        assert!(d.session_description(0).is_empty());
        let mut out = Vec::new();
        assert!(d.encrypt(b"opus", &mut out));
        assert_eq!(out, b"opus");
        assert_eq!(
            d.decrypt(2, b"opus", now).as_deref(),
            Some(b"opus".as_slice())
        );
    }

    #[test]
    fn version_one_sends_key_package_and_blocks_plain_audio() {
        let now = Instant::now();
        let mut d = Dave::new(1, 10).expect("oturum");
        let out = d.session_description(1);
        assert!(matches!(out.as_slice(), [Outgoing::Binary(b)] if b[0] == binary_op::KEY_PACKAGE));
        let mut buf = Vec::new();
        assert!(!d.encrypt(b"opus", &mut buf), "anahtar yokken gönderilmez");
        assert_eq!(
            d.decrypt(2, b"opus", now),
            None,
            "şifresiz kare kabul edilmez"
        );
    }

    #[test]
    fn transitions_send_ready_except_zero() {
        let now = Instant::now();
        let mut d = Dave::new(1, 10).expect("oturum");
        d.session_description(1);
        assert_eq!(
            d.prepare_transition(5, 0, now),
            vec![Outgoing::Json(protocol::transition_ready(5))]
        );
        // Sürüm 0'a geçiş hazırlığında şifresiz kareler kabul.
        assert!(d.decrypt(2, b"opus", now).is_some());
        d.execute_transition(5, now);
        assert_eq!(d.version(), 0);
        let mut out = Vec::new();
        assert!(d.encrypt(b"opus", &mut out));
        assert!(
            d.prepare_transition(0, 0, now).is_empty(),
            "0 hemen yürütülür"
        );
        // Bilinmeyen geçiş yok sayılır.
        d.execute_transition(99, now);
    }

    #[test]
    fn bad_commit_triggers_recovery() {
        let now = Instant::now();
        let mut d = Dave::new(1, 10).expect("oturum");
        d.session_description(1);
        let out = d.binary(binary_op::ANNOUNCE_COMMIT, &[0, 4, 0xde, 0xad], now);
        assert_eq!(out[0], Outgoing::Json(protocol::invalid_commit_welcome(4)));
        assert!(matches!(&out[1], Outgoing::Binary(b) if b[0] == binary_op::KEY_PACKAGE));
        // Bozuk yükler panik yapmaz.
        for op in 25..=31 {
            let _ = d.binary(op, &[0xff; 3], now);
            let _ = d.binary(op, &[], now);
        }
    }

    /// İki istemci, sahte sunucu: tam DAVE akışı ve şifreli kare alışverişi.
    #[test]
    fn full_handshake_and_encrypted_audio() {
        use crate::dave::session::tests::{CHANNEL, Server};
        let now = Instant::now();
        let srv = Server::new();
        let mut a = Dave::new(1, CHANNEL).expect("A");
        let mut b = Dave::new(2, CHANNEL).expect("B");
        for d in [&mut a, &mut b] {
            d.clients_connect(&[1, 2]);
        }
        // op 4 (sürüm 1) → op 26; op 25.
        let kp = |out: Vec<Outgoing>| match out.as_slice() {
            [Outgoing::Binary(b)] if b[0] == binary_op::KEY_PACKAGE => b[1..].to_vec(),
            other => panic!("anahtar paketi bekleniyordu: {other:?}"),
        };
        let _kp_a = kp(a.session_description(1));
        let kp_b = kp(b.session_description(1));
        assert!(
            a.binary(binary_op::EXTERNAL_SENDER, &srv.sender_bytes, now)
                .is_empty()
        );
        assert!(
            b.binary(binary_op::EXTERNAL_SENDER, &srv.sender_bytes, now)
                .is_empty()
        );

        // op 27 → A op 28 (commit + welcome).
        let out = a.binary(binary_op::PROPOSALS, &srv.append_add(0, &[kp_b]), now);
        let [Outgoing::Binary(cw)] = out.as_slice() else {
            panic!("commit bekleniyordu: {out:?}");
        };
        assert_eq!(cw[0], binary_op::COMMIT_WELCOME);
        let (commit, welcome) = crate::dave::session::split_commit_welcome(&cw[1..]).expect("ayır");
        let with_tid = |p: &[u8]| {
            let mut v = vec![0, 7];
            v.extend_from_slice(p);
            v
        };
        // op 29 → A, op 30 → B: ikisi de op 23 gönderir.
        let ready = vec![Outgoing::Json(protocol::transition_ready(7))];
        assert_eq!(
            a.binary(binary_op::ANNOUNCE_COMMIT, &with_tid(commit), now),
            ready
        );
        assert_eq!(b.binary(binary_op::WELCOME, &with_tid(welcome), now), ready);

        // Geçiş yürütülmeden kendi anahtarımız yok.
        let mut frame = Vec::new();
        assert!(!a.encrypt(b"merhaba", &mut frame));
        // op 22.
        a.execute_transition(7, now);
        b.execute_transition(7, now);
        assert!(a.encrypt(b"merhaba", &mut frame));
        assert_ne!(frame.as_slice(), b"merhaba".as_slice());
        assert_eq!(&frame[frame.len() - 2..], &crate::dave::frame::MAGIC);
        assert_eq!(
            b.decrypt(1, &frame, now).as_deref(),
            Some(b"merhaba".as_slice())
        );
        // Ters yön ve tekrar koruması.
        assert!(b.encrypt(b"selam", &mut frame));
        assert_eq!(
            a.decrypt(2, &frame, now).as_deref(),
            Some(b"selam".as_slice())
        );
        assert_eq!(a.decrypt(2, &frame, now), None);
        // Şifresiz kare artık kabul edilmez.
        assert_eq!(
            a.decrypt(2, b"ham", now + std::time::Duration::from_secs(11)),
            None
        );
    }
}
