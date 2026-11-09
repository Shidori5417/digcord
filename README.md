# Digcord

> Discord için tamamen Rust ile yazılmış, hızlı ve hafif bir terminal (TUI) istemcisi.

## Ne yapıyor?

Digcord, resmi Discord istemcisinin yerine terminalde çalışan bir istemcidir:

- Sunucular, kanallar, kategoriler, thread'ler
- Mesaj okuma / yazma / düzenleme / silme / yanıt / tepki (reaction)
- DM ve grup DM'ler, arkadaş listesi
- Okunmamış sayaçları, mention vurgusu, masaüstü bildirimleri
- Terminal içinde resim, emoji ve sticker gösterimi (Kitty / Sixel / iTerm2 / yarım blok yedeği)
- Sesli kanallara katılma (DAVE uçtan uca şifreleme ile)

Gateway ve REST katmanı **sıfırdan** yazılır; `twilight`, `serenity` gibi Discord kütüphaneleri kullanılmaz.
Genel amaçlı altyapı kütüphaneleri (tokio, rustls, WebSocket, HTTP, kripto) kullanılır.

## ⚠️ Önemli uyarı

Digcord **kullanıcı token'ı** ile çalışır. Kullanıcı hesabıyla üçüncü taraf istemci kullanmak
Discord Hizmet Koşulları'na aykırıdır ve **hesabın kalıcı olarak kapatılmasına** yol açabilir.

- Geliştirme ve testleri **ana hesabında değil, yan hesapta** yap.
- İstemci, resmi istemciden ayırt edilemeyecek şekilde davranmaya çalışır ama bu bir garanti değildir.
- Spam, otomasyon (self-bot) ve toplu işlem özellikleri bilinçli olarak **eklenmeyecek** — hem ban riskini artırır hem projenin amacı bu değil.

## Hedef platform

- Birincil: Linux (CachyOS / Arch, KDE Plasma, PipeWire)
- Terminal: Kitty, WezTerm, Konsole, foot, Alacritty (Alacritty'de resim yok → yarım blok yedeği)
- İkincil (sonra): macOS, Windows

## Teknoloji

| Alan | Seçim |
|---|---|
| Dil | Rust (stable, edition 2024) |
| Async | tokio |
| TUI | ratatui + crossterm |
| Resim | ratatui-image (+ `image` crate) |
| WebSocket | tokio-tungstenite + rustls |
| HTTP | hyper (veya reqwest, rustls ile) |
| Sıkıştırma | zlib-stream (flate2 + miniz_oxide, saf Rust) |
| JSON | serde + serde_json (sıcak yollarda simd-json opsiyonel) |
| Token saklama | keyring (Secret Service → KWallet) |
| Bildirim | notify-rust |
| Ses | cpal (G/Ç), opus, openmls + RustCrypto (DAVE) |
| Yerel önbellek | rusqlite veya redb |
| Log | tracing + tracing-appender (dosyaya, TUI'yi bozmadan) |

Ayrıntılar: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

## Dokümanlar

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — workspace yapısı, veri akışı, katmanlar
- [`docs/PROTOCOL_NOTES.md`](docs/PROTOCOL_NOTES.md) — gateway, REST, kullanıcı token'ı ayrıntıları, ses/DAVE
- [`docs/TUI_DESIGN.md`](docs/TUI_DESIGN.md) — ekran düzeni, kısayollar, config
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — aşamalar ve kabul kriterleri
- [`CLAUDE.md`](CLAUDE.md) — kodlama ajanı için proje kuralları
- [`PROMPT.md`](PROMPT.md) — başlangıç promptu
- [`docs/GIT_WORKFLOW.md`](docs/GIT_WORKFLOW.md) — GitHub, branch, commit ve push kuralları

## Lisans

**Tescilli — Tüm hakları saklıdır.** Kod açık kaynak değildir; kullanılamaz, kopyalanamaz,
değiştirilemez, dağıtılamaz. Ayrıntılar: [`LICENSE`](LICENSE)
