# Mimari

## Temel ilkeler

1. **Katmanlar birbirini bilmez.** TUI, ağ kodunu doğrudan çağırmaz; mesajlaşma kanalları üzerinden konuşur.
2. **Tek doğruluk kaynağı: Store.** Gateway olayları store'u günceller, TUI store'dan çizer.
3. **UI asla bloklanmaz.** Ağ, disk, resim çözme ve ses ayrı task/thread'lerde çalışır.
4. **Önce doğruluk, sonra hız.** Ama sıcak yollar (mesaj listesi çizimi, gateway ayrıştırma) baştan ölçülerek yazılır.
5. **`unsafe` yok** (ses sürücüsü bağlamaları hariç, gerekçesiyle belgelenerek).

## Cargo workspace

```
digcord/
├── Cargo.toml                  # [workspace]
├── crates/
│   ├── digcord-model/           # Discord veri tipleri (Snowflake, Message, Channel, Guild, User...)
│   ├── digcord-http/            # REST istemcisi + rate limit bucket yönetimi
│   ├── digcord-gateway/         # WebSocket gateway: identify, heartbeat, resume, zlib-stream
│   ├── digcord-store/           # Bellek içi durum + yerel önbellek (SQLite/redb)
│   ├── digcord-media/           # CDN indirme, resim çözme/ölçekleme, disk önbelleği
│   ├── digcord-voice/           # Ses gateway'i, UDP, RTP, Opus, DAVE (MLS) şifreleme
│   ├── digcord-notify/          # Masaüstü bildirimleri, okunmamış/mention hesaplama
│   └── digcord-tui/             # ratatui arayüzü, girdi, kısayollar
└── src/main.rs                 # binary: config yükle, katmanları başlat, bağla
```

Neden ayrı crate'ler: derleme süresi, test izolasyonu ve ileride `digcord-gateway`/`digcord-http`'yi
başka projelerde (ör. diğer Discord araçların) tekrar kullanabilmek.

## Veri akışı

```
           ┌──────────────┐  GatewayEvent   ┌──────────────┐  StoreChange  ┌──────────────┐
Discord ──▶│   gateway    │───────────────▶│    store     │──────────────▶│     tui      │
  WS       └──────────────┘                 └──────────────┘   (notify)    └──────┬───────┘
                                                  ▲                               │ Action
           ┌──────────────┐  Response             │                               ▼
Discord ◀─▶│     http     │◀──────────────────────┴────────────────────── ┌──────────────┐
  REST     └──────────────┘          istek (gönder, düzenle, ack...)       │  dispatcher  │
                                                                           └──────────────┘
```

- **gateway → store:** `tokio::mpsc` ile tiplenmiş olaylar (`MessageCreate`, `ChannelUpdate`...).
- **store → tui:** Store bir "değişti" sinyali yayınlar (`tokio::sync::watch` veya küçük bir sürüm sayacı); TUI bir sonraki karede yeniden çizer. Her olayda çizim yapılmaz → kare hızı sınırlı (ör. en fazla 60 fps, boşta 0).
- **tui → dispatcher:** Kullanıcı eylemleri (`Action::SendMessage{..}`) dispatcher'a gider; dispatcher HTTP/gateway çağrısı yapar, **iyimser güncelleme** (optimistic update) ile store'a "gönderiliyor" mesajı ekler, yanıt gelince nonce ile eşleştirir.

## Katmanlar

### digcord-model
- `Snowflake(u64)` — zaman damgası çıkarma yardımcıları.
- Serde tipleri; bilinmeyen alanlar sessizce yok sayılır (Discord sık alan ekler).
- Büyük metinler için `Box<str>` / `Arc<str>`; tekrarlanan kullanıcı adları için interning düşünülebilir.

### digcord-http
- Tek `hyper` istemcisi, HTTP/2, keep-alive.
- **Rate limit:** `X-RateLimit-Bucket`, `-Remaining`, `-Reset-After` başlıklarına göre bucket haritası; route → bucket eşlemesi öğrenilir. Global 429 için ortak kilit.
- Kullanıcı istemcisi başlıkları (`User-Agent`, `X-Super-Properties`, vb.) tek yerde üretilir — bkz. PROTOCOL_NOTES.
- Tüm uç noktalar tiplenmiş fonksiyonlar: `create_message`, `edit_message`, `ack_message`, `get_messages(before/after/around)` ...

### digcord-gateway
- `tokio-tungstenite` + `rustls`.
- `zlib-stream` sıkıştırma: tek kalıcı `flate2::Decompress`, `00 00 ff ff` son ekiyle çerçeve tamamlama.
- Durum makinesi: `Connecting → Hello → Identify/Resume → Ready → Running → Reconnecting`.
- Heartbeat ayrı task; ACK gelmezse bağlantı "zombi" sayılıp resume edilir.
- Resume: `session_id` + `seq` + `resume_gateway_url`. Başarısızsa tam identify.
- Olay ayrıştırma: önce `{"op","t","s"}` başlığı, sonra `d` sadece gerekli tipe ayrıştırılır (bilinmeyen olaylar atlanır → CPU tasarrufu).

### digcord-store
- Bellek içi: `HashMap<Snowflake, Guild>`, kanal başına `VecDeque<Message>` (sınırlı pencere, ör. son 200).
- Okunmamış durumu: `read_state` (READY'den) + yeni mesajlar.
- Kalıcı önbellek: son açılan kanalların mesajları ve guild/kanal listesi → açılışta anında görüntü, sonra gateway ile senkron.
- Store tek task'ta sahiplenilir (actor modeli) **veya** `parking_lot::RwLock` ile paylaşılır — ilk aşamada actor önerilir (kilit derdi yok).

### digcord-media
- CDN istekleri ayrı bir havuzda, eşzamanlılık sınırı (ör. 4).
- Çözülmüş resimler hedef hücre boyutuna ölçeklenip LRU'da tutulur; ham dosyalar `~/.cache/digcord/media` altında.
- Animasyonlu GIF/APNG/WebP: ilk aşamada sadece ilk kare.
- Terminal yeteneği açılışta bir kez algılanır (`ratatui-image` picker).

### digcord-voice
Bkz. PROTOCOL_NOTES "Ses" bölümü. Kendi thread'inde (gerçek zamanlı ses tokio'ya bırakılmaz):
- Ses gateway WebSocket (ayrı, v8)
- UDP soketi, IP keşfi, RTP paketleme
- Transport şifreleme (`aead_aes256_gcm_rtpsize` / `aead_xchacha20_poly1305_rtpsize`)
- **DAVE:** MLS grup anahtar değişimi (openmls) + kare başına şifreleme. Mart 2026'dan beri zorunlu; olmadan bağlantı 4017 ile reddedilir.
- cpal ile mikrofon/hoparlör, Opus kodlama/çözme, jitter buffer.

### digcord-tui
- Bkz. `TUI_DESIGN.md`.
- Bileşen ağacı: `App { sidebar, channel_list, message_view, input, member_list, popup_stack }`.
- Mesaj görünümü sanal kaydırma ile: sadece görünür satırlar ölçülür ve çizilir; satır yüksekliği (wrap sonrası) mesaj başına önbelleğe alınır, genişlik değişince geçersizlenir.

## Eşzamanlılık modeli

| Bileşen | Çalıştığı yer |
|---|---|
| Gateway okuma/yazma | tokio task |
| HTTP istekleri | tokio task'ları |
| Store | tek tokio task (actor) |
| TUI çizim + girdi | ana thread (crossterm event stream) |
| Resim çözme | `spawn_blocking` / rayon |
| Ses | ayrı OS thread'leri (yakalama, çalma, ağ) |

## Hata yönetimi

- Crate'lerde `thiserror` ile tiplenmiş hatalar, binary'de `anyhow`/`color-eyre`.
- Ağ hataları kullanıcıya durum çubuğunda gösterilir, uygulama çökmez.
- `panic` olursa terminal mutlaka eski haline döndürülür (panic hook).

## Güvenlik

- Token **asla** düz metin config'e veya log'a yazılmaz; `keyring` ile saklanır.
- Log'larda `Authorization` başlığı ve token benzeri dizeler maskelenir.
- Medya dosyaları sadece Discord CDN alan adlarından indirilir.
