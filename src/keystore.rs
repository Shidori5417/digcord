//! Token'ı sistem anahtarlığında (Secret Service: gnome-keyring / KWallet) saklar.
//! Tüm fonksiyonlar D-Bus üzerinden bloklayıcıdır: UI thread'inde değil,
//! `spawn_blocking` içinde çağrılmalı.

use digcord_model::Token;
use keyring_core::{Entry, Error};

const SERVICE: &str = "digcord";
const USER: &str = "discord-token";

/// Keyring hatasını güvenli bir mesaja çevirir. Kaynak hata **bilerek** taşınmaz:
/// `BadDataFormat` gibi varyantlar saklanan veriyi (token'ı) içerebilir.
fn map(e: Error) -> anyhow::Error {
    anyhow::anyhow!(
        "anahtarlık: {}",
        match e {
            Error::NoStorageAccess(_) =>
                "erişilemedi (Secret Service çalışıyor ve kilidi açık mı?)",
            Error::PlatformFailure(_) => "platform hatası",
            Error::BadEncoding(_) | Error::BadDataFormat(..) => "kayıtlı değer bozuk",
            Error::NoDefaultStore => "anahtarlık kurulmadı",
            _ => "beklenmeyen hata",
        }
    )
}

/// Secret Service bağlantısını varsayılan depo olarak kurar. Bir kez çağrılır.
pub fn init() -> anyhow::Result<()> {
    let store = zbus_secret_service_keyring_store::Store::new().map_err(map)?;
    keyring_core::set_default_store(store);
    Ok(())
}

fn entry() -> anyhow::Result<Entry> {
    Entry::new(SERVICE, USER).map_err(map)
}

pub fn load() -> anyhow::Result<Option<Token>> {
    match entry()?.get_password() {
        Ok(p) => Ok(Token::new(&p)),
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(map(e)),
    }
}

pub fn save(token: &Token) -> anyhow::Result<()> {
    entry()?.set_password(token.expose()).map_err(map)
}

pub fn delete() -> anyhow::Result<()> {
    match entry()?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(map(e)),
    }
}
