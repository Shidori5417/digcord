# CLAUDE.md — Digcord proje kuralları

Bu dosya projede çalışan kodlama ajanı (Claude Code vb.) içindir. Her oturumda önce bunu,
sonra ilgili `docs/` dosyasını oku.

## Proje
Rust ile yazılmış, kullanıcı token'ıyla çalışan Discord TUI istemcisi.
Gateway, REST ve ses katmanı **sıfırdan**; twilight/serenity/songbird gibi Discord kütüphaneleri **kullanma**.

## Okuma sırası
1. `README.md` — kapsam ve teknoloji
2. `docs/ARCHITECTURE.md` — crate yapısı ve veri akışı (bu yapıdan sapma; gerekiyorsa önce öner)
3. `docs/ROADMAP.md` — **şu an hangi aşamadaysak sadece onu yap**
4. `docs/PROTOCOL_NOTES.md` — ağ kodu yazmadan önce
5. `docs/TUI_DESIGN.md` — arayüz kodu yazmadan önce

## Kod kuralları
- Rust stable, edition 2024. `cargo fmt` ve `cargo clippy --all-targets -- -D warnings` temiz olmalı.
- `unsafe` yasak (gerekirse gerekçesiyle yorum + kullanıcıya sor).
- `unwrap()`/`expect()` sadece testlerde ve gerçekten imkânsız durumlarda (yorumla açıkla).
- Kütüphane crate'lerinde `thiserror`, binary'de `anyhow`/`color-eyre`.
- UI thread'inde ağ, disk veya ağır hesaplama yapma.
- Sıcak yollarda gereksiz `clone()`, `String` tahsisi ve `format!` kullanma; ölçmeden "optimize" etme.
- Her yeni bağımlılık için kısa gerekçe yaz; ağır bağımlılıklardan kaçın.
- Yorumlar ve commit mesajları Türkçe olabilir; tanımlayıcılar İngilizce.

## Güvenlik
- Token'ı **asla** log'a, panic mesajına, config'e veya test fixture'ına yazma.
- Gerçek gateway/REST kayıtlarını fixture yaparken kullanıcı adı, ID, token ve mesaj içeriklerini anonimleştir.
- Self-bot, otomatik yanıt, toplu işlem veya captcha atlatma kodu **yazma**.

## Protokol
- Belgelenmemiş bir alan/uç nokta kullanmadan önce `PROTOCOL_NOTES.md`'ye ekle ve
  "doğrulanmadı" işaretle. Kullanıcı resmi istemci trafiğiyle doğrulayınca işaret kaldırılır.
- İstemci kimliği (super properties, User-Agent) sadece `ClientProperties` içinden üretilir.

## Test
- Ayrıştırıcılar, rate limit, durum makinesi ve DAVE için birim testleri zorunlu.
- Ağ gerektiren testler `#[ignore]` + `DIGCORD_TEST_TOKEN` ortam değişkeni ile, sadece yan hesap.
- TUI bileşenleri için `ratatui` `TestBackend` ile snapshot testleri (`insta`).

## Git ve GitHub
Ayrıntılar: `docs/GIT_WORKFLOW.md`. Özet:
- Depo **private**. Görünürlüğü asla kendin değiştirme.
- Her aşama kendi branch'inde (`asama-N-...`). `main`'e sadece kabul kriterleri sağlanınca birleşir.
- Her çalışan küçük adımdan sonra commit: fmt + clippy + test temiz olmalı.
- Commit mesajı Conventional Commits, açıklama Türkçe: `feat(gateway): ...`.
- Her commit'ten sonra push et; aşama bitince etiket at (`v0.N.0`).
- Push, merge ve etiket öncesi kullanıcıya ne yapacağını tek satırla söyle.
- Token, `.env` ve `/captures/` asla commit'lenmez; `git add` öncesi `git status` ile kontrol et.

## Lisans
- Proje **tescilli** (`LICENSE`): tüm hakları saklıdır, açık kaynak değil.
- Her crate'in `Cargo.toml`'unda: `license-file = "LICENSE"` (workspace'ten miras) ve `publish = false`.
- Açık kaynak lisanslı kodu kopyala-yapıştır yapma; bağımlılık olarak kullan. GPL/AGPL
  bağımlılık ekleme (tescilli projeyle uyumsuz) — MIT/Apache/BSD/ISC/Zlib uygun.

## Çalışma şekli
- Bir aşamaya başlamadan önce kısa plan yaz, kullanıcının onayını al.
- Her aşama sonunda: kabul kriterlerini tek tek kontrol et, `ROADMAP.md`'de işaretle.
- Emin olmadığın protokol davranışında tahmin etme; kullanıcıdan DevTools ile doğrulama iste.
