# Protokol Notları

> Discord'un kullanıcı istemcisi protokolü **belgelenmemiştir** ve haber vermeden değişir.
> Buradaki bilgiler başlangıç noktasıdır; her alan uygulanmadan önce resmi istemcinin
> ağ trafiği (tarayıcı DevTools → Network/WS) ile **yan hesapta** doğrulanmalıdır.
> Değişen her şey bu dosyaya tarihle not edilir.

## 1. Kimlik doğrulama

- Token kullanıcıdan alınır (ilk açılışta TUI içinde maskeli giriş alanı) ve `keyring` ile saklanır.
- E-posta/şifre ile giriş, captcha ve 2FA akışları **ilk aşamada yok** — captcha'yı otomatik geçmeye çalışmak yasak/riskli; sadece token ile giriş.
- REST'te başlık: `Authorization: <token>` (kullanıcı token'ında `Bot ` öneki **yok**).
- Token geçersizse (401 / gateway close 4004) → token silinir, giriş ekranına dönülür.

## 2. İstemci kimliği (super properties)

Resmi istemci her istekte ve IDENTIFY'da istemci bilgisini gönderir. Tutarsız değerler
hesabı işaretletebilir; **hepsi tek bir `ClientProperties` yapısından** üretilmeli:

- `os`, `browser`, `device`, `system_locale`, `browser_user_agent`, `browser_version`,
  `os_version`, `release_channel`, `client_build_number` ...
- REST: `X-Super-Properties` = bu yapının JSON'unun base64'ü.
- `User-Agent` IDENTIFY'daki `browser_user_agent` ile **aynı** olmalı.
- `client_build_number` zamanla eskir → config'den güncellenebilir olsun.
- Tek tutarlı profil seç (ör. "Linux + Chrome web istemcisi") ve ona sadık kal.

## 3. Gateway

- URL: `GET /gateway` → `wss://gateway.discord.gg/?v=9&encoding=json&compress=zlib-stream`
  (kullanıcı istemcileri pratikte v9 kullanıyor; doğrula).
- **Opcodes (temel):** 0 Dispatch, 1 Heartbeat, 2 Identify, 3 Presence Update, 4 Voice State Update,
  6 Resume, 7 Reconnect, 9 Invalid Session, 10 Hello, 11 Heartbeat ACK.
- Kullanıcı istemcisine özgü: guild'e abone olma / üye listesi istekleri için ek opcode'lar var
  (ör. lazy guild / guild subscriptions) — trafikten çıkar ve belgeye ekle.
- **IDENTIFY (kullanıcı):** `token`, `capabilities` (bit alanı), `properties` (super properties),
  `presence`, `compress: false`, `client_state`. `intents` gönderilmez.
- **READY** çok büyüktür (tüm guild'ler, kanallar, read_state, ilişkiler, ayarlar).
  `READY_SUPPLEMENTAL` ayrıca gelir. Ayrıştırma akış halinde / seçici yapılmalı.
- Heartbeat: `heartbeat_interval * jitter` ile ilk vuruş, ACK gelmezse kapat + resume.
- **Close kodları:** 4004 (kimlik hatası → resume yok), 4007/4009 (yeniden identify),
  diğerlerinin çoğu resume edilebilir. Tablo `digcord-gateway` içinde tek yerde tutulsun.
- Yeniden bağlanma: üstel geri çekilme + jitter, en fazla ~1 dk.

## 4. REST

- Taban: `https://discord.com/api/v9`
- Mesaj gönderme: `POST /channels/{id}/messages` — `nonce` (istemci üretimi snowflake) ile
  iyimser güncelleme eşleştirmesi.
- Okundu bilgisi: `POST /channels/{id}/messages/{msg}/ack` (+ token alanı, doğrula).
- Geçmiş: `GET /channels/{id}/messages?before=&limit=50`
- Yazıyor: `POST /channels/{id}/typing` (en fazla ~8 sn'de bir).
- Ek dosya: multipart veya önce yükleme URL'si alıp sonra gönderme akışı (resmi istemcinin
  kullandığını doğrula).
- **Rate limit:** bucket başlıklarına uy; 429'da `retry_after` kadar bekle. Kullanıcı hesaplarında
  limitler botlardan sıkı olabilir; agresif önceden yükleme yapma.

## 5. Davranış kuralları (ban riskini azaltmak için)

- Resmi istemcinin **yapmadığı** şeyi yapma: toplu geçmiş indirme, otomatik yanıt, çok hızlı
  kanal gezme, gizli API uçları.
- Görünür olmayan kanallar için mesaj çekme / abonelik yapma.
- Yazıyor/okundu/presence davranışı resmi istemciye benzesin (ayarlanabilir: "okundu gönderme" kapatılabilir).

## 6. CDN ve medya

- Avatar/emoji/ek URL'leri `cdn.discordapp.com` / `media.discordapp.net`.
- Boyut parametresiyle (`?size=64`, `?width=&height=`) küçük sürüm iste → bant genişliği tasarrufu.
- Ek URL'leri imzalı ve süreli olabilir; süresi dolanlar yeniden çözülmeli (refresh uç noktası).

## 7. Ses

Ses en zor ve en kırılgan kısım; ayrı aşamada (bkz. ROADMAP) ele alınır.

### Akış
1. Ana gateway'e **op 4 Voice State Update** (`guild_id`, `channel_id`, `self_mute`, `self_deaf`).
2. `VOICE_STATE_UPDATE` (session_id) + `VOICE_SERVER_UPDATE` (token, endpoint) beklenir.
3. Ses gateway'ine bağlan (`wss://{endpoint}/?v=8`), Identify (op 0) → Ready (op 2: ssrc, ip, port, modes).
4. UDP soketi aç, **IP keşfi** (74 baytlık paket) yap.
5. Select Protocol (op 1) — şifreleme modu: `aead_aes256_gcm_rtpsize` (tercih) veya
   `aead_xchacha20_poly1305_rtpsize`. Eski modlar artık kabul edilmiyor.
6. Session Description (op 4) → secret_key.
7. Speaking (op 5), sonra RTP + Opus paketleri (20 ms kareler, 48 kHz stereo).

### DAVE (uçtan uca şifreleme) — ZORUNLU
- Discord, **1–2 Mart 2026'dan beri** DAVE desteklemeyen istemcileri DM, grup DM ve sunucu ses
  kanallarına kabul etmiyor; ses gateway'i bağlantıyı **close code 4017** ile reddediyor.
  (Stage kanalları istisna.)
- DAVE: grup anahtar değişimi **MLS** ile, her gönderenin her Opus karesi kendi anahtarıyla
  şifrelenir (transport şifrelemesinin **içinde**, ikinci katman).
- Ses gateway'inde DAVE'e özel opcode'lar (protokol sürümü, MLS key package, commit/welcome,
  geçiş hazır/yürüt) vardır — referans: Discord'un açık kaynak `dave-protocol` beyaz kağıdı
  ve `libdave` (C++) referans uygulaması.
- Rust tarafı: `openmls` (MLS) + RustCrypto (AES-GCM). Kare formatı, sentinel/ek veri ve
  "codec'e duyarlı" şifrelenmeyen baytlar beyaz kağıda **birebir** uyulmalı.
- Strateji: önce DAVE olmadan mimariyi kurmak mümkün değil (4017) — bu yüzden ses aşamasının
  ilk alt hedefi **DAVE el sıkışmasının birim testleri** (libdave test vektörleriyle) olmalı.

### Ses G/Ç
- cpal (PipeWire/Pulse/ALSA arka uçları), 48 kHz.
- Opus: `opus` crate (libopus bağlaması) — saf Rust alternatifi olgunsa değerlendirilebilir.
- Jitter buffer, kullanıcı başına ses seviyesi, push-to-talk / ses algılama (VAD).
- Gürültü bastırma (opsiyonel, sonra): `nnnoiseless` (RNNoise'un Rust portu).

## Kaynaklar (doğrulanacak)

- Discord resmi geliştirici dokümanları (bot API'si; kullanıcı API'sine çoğu benzer).
- Topluluk kullanıcı API'si dokümanları (ör. "discord-userdoccers").
- `discord/dave-protocol` ve `discord/libdave` depoları.
- Mevcut TUI istemcileri (discordo — Go, Endcord — Python) **sadece davranış referansı** için.
