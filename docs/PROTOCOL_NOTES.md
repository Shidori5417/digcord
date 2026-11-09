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
  — ✅ doğrulandı 2026-11-09, bkz. §3.1. Resmi web istemcisi `GET /gateway` çağırmadan sabit
  URL'ye bağlanıyor gibi görünüyor (REST trafiği ayrıca kontrol edilmedi — doğrulanmadı).
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

### 3.1 Gözlenen trafik — 2026-11-09 (Linux, Chrome 155, web istemcisi, yan hesap)

Kaynak: CDP ile `discord.com/app` yüklenirken WebSocket kaydı. Ham kayıt `/captures/` altında
(git'e girmez). Token ve kişisel veriler bu notta yok.

**Bağlantı — ✅ doğrulandı**
- URL: `wss://gateway.discord.gg/?encoding=json&v=9&compress=zlib-stream` (parametre sırası bu).
- El sıkışma başlıkları: `Origin: https://discord.com`, `User-Agent` (IDENTIFY'daki
  `browser_user_agent` ile aynı), `Accept-Language`, `Cache-Control/Pragma: no-cache`,
  `Sec-WebSocket-Extensions: permessage-deflate; client_max_window_bits`. Cookie gönderilmiyor.
  - Not: `tokio-tungstenite` permessage-deflate desteklemiyor; bu başlığı sunmamak küçük bir parmak
    izi farkı. Sunup müzakere etmemek mümkün mü — **doğrulanmadı**.
- Sunucu → istemci çerçeveleri binary, tek kalıcı zlib akışı (`78 da` başlığı ile başlıyor), her mesaj
  `00 00 ff ff` ile bitiyor. İstemci → sunucu çerçeveleri sıkıştırılmamış JSON metin.
- Hello: `{"op":10,"d":{"heartbeat_interval":41250,"_trace":[...]}}`.

**IDENTIFY (op 2) — ✅ gözlendi, alan kaynakları kısmen doğrulanmadı**
```json
{"op":2,"d":{
  "token":"…","capabilities":1767421,
  "properties":{
    "os":"Linux","browser":"Chrome","device":"","device_model_name":"","system_locale":"tr-TR",
    "has_client_mods":false,"browser_user_agent":"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/155.0.0.0 Safari/537.36",
    "browser_version":"155.0.0.0","os_version":"","referrer":"","referring_domain":"",
    "referrer_current":"","referring_domain_current":"","release_channel":"stable",
    "client_build_number":667580,"client_event_source":null,
    "client_launch_id":"<uuid v4>","launch_signature":"<uuid biçimli>",
    "client_app_state":"unfocused","is_fast_connect":false,
    "gateway_connect_reasons":"AppSkeleton","installation_id":"<snowflake>.<imza>"},
  "presence":{"status":"unknown","since":0,"activities":[],"afk":false},
  "compress":false,"client_state":{"guild_versions":{}},
  "qos_token":"<base64>"}}
```
- `intents` yok (beklendiği gibi).
- **Doğrulanmadı:** `launch_signature`, `installation_id` ve `qos_token` nereden geliyor?
  `qos_token` READY'de de dönüyor (muhtemelen önceki oturumdan saklanıyor); `installation_id`
  muhtemelen bir REST yanıtından/yerel depodan. İlk canlı bağlantıdan önce bu alanların gönderilip
  gönderilmeyeceği kararlaştırılmalı.

**IDENTIFY sonrası istemcinin gönderdikleri — ✅ gözlendi, anlamı doğrulanmadı**
- op 4 Voice State Update (tüm alanlar null/false, `self_mute: true`, `flags: 0`).
- op 3 Presence Update (kullanıcının kayıtlı durumu ve özel durum etkinliği, `type: 4`).
- op 41: `{"initialization_timestamp":<ms>,"session_id":"<uuid>","client_launch_id":"<uuid>"}`.
- op 40 **QoS heartbeat** — ✅ doğrulandı (100 sn'lik ikinci kayıt):
  `{"seq":<son s>,"qos":{"active":false,"ver":31,"reasons":[]}}`, sunucu op 11 ile yanıtlıyor.
  Web istemcisi **op 1 hiç göndermiyor**; heartbeat tamamen op 40 ile yapılıyor:
  READY'den hemen sonra bir tane, sonra ilk vuruş `interval × jitter` (~16 sn), ardından her
  `heartbeat_interval` (~41 sn). Sunucu op 1 isterse yine de hemen yanıt verilmeli (protokol gereği).
- İstemci zaman zaman (ör. ~90. sn) op 3 presence tekrar gönderiyor — muhtemelen odak/boşta
  durumu değişimi; **doğrulanmadı**.

**Digcord'un resmi web istemcisinden bilinen farkları (Aşama 1)** — her biri bilinçli, karar bekliyor:
- IDENTIFY'da `launch_signature`, `installation_id`, `qos_token` **gönderilmiyor** (kaynakları bilinmiyor).
- El sıkışmada `permessage-deflate` sunulmuyor (tungstenite desteklemiyor).
- READY sonrası op 4 / op 3 / op 41 **gönderilmiyor**; sadece op 40 gönderiliyor.
- RESUME (op 6) biçimi resmi istemciden **gözlenmedi**; standart `{token, session_id, seq}`.
- `client_build_number` sabit (667580); zamanla eskir, config desteği sonraki aşamada.
- (Aşama 2) REST'te `X-Installation-ID` **gönderilmiyor**; `X-Super-Properties`'te `launch_signature` yok.
- (Aşama 2) Kanal açılınca op 37 ve op 8 gönderiliyor, **op 43 gönderilmiyor**.
- (Aşama 2) Resmi istemcinin kanal açılışında yaptığı diğer REST istekleri (`GET /channels/{id}`,
  `/science`, `/users/@me/...` vb.) yapılmıyor; sadece mesaj geçmişi.

**READY — ✅ gözlendi** (`v: 9`, ~357 KB açılmış, ~53 KB sıkıştırılmış)
- Üst anahtarlar: `user`, `users`, `guilds`, `private_channels`, `relationships`, `read_state`
  (`{entries, partial, version}`), `user_guild_settings`, `merged_members`, `session_id`,
  `session_type`, `resume_gateway_url` (sorgu parametresiz, ör. `wss://gateway-us-east1-d.discord.gg`),
  `auth_session_id_hash`, `analytics_token`, `qos_token`, `experiments`, …
- Guild nesnesi kullanıcı istemcisi biçiminde: ad, ikon vb. `properties` alt nesnesinde; `channels`,
  `roles`, `emojis`, `threads`, `member_count`, `lazy`, `data_mode`, `version` üst düzeyde.
- `private_channels[]`: `recipient_ids` (kullanıcı nesneleri ayrı `users` dizisinde), `type`, `last_message_id`.
- Ardından `READY_SUPPLEMENTAL` (`guilds`, `merged_presences`, `merged_members`, `lazy_private_channels`…).

## 4. REST

- Taban: `https://discord.com/api/v9`
- Mesaj gönderme: `POST /channels/{id}/messages` — `nonce` (istemci üretimi snowflake) ile
  iyimser güncelleme eşleştirmesi.
- Okundu bilgisi: `POST /channels/{id}/messages/{msg}/ack` (+ token alanı, doğrula).
- Geçmiş: `GET /channels/{id}/messages?limit=10` (kanal açılışı), yukarı kaydırınca
  `?before=<en eski id>&limit=20` — ✅ gözlendi 2026-11-09, bkz. §4.1.
- Yazıyor: `POST /channels/{id}/typing` (en fazla ~8 sn'de bir).
- Ek dosya: multipart veya önce yükleme URL'si alıp sonra gönderme akışı (resmi istemcinin
  kullandığını doğrula).
- **Rate limit:** bucket başlıklarına uy; 429'da `retry_after` kadar bekle. Kullanıcı hesaplarında
  limitler botlardan sıkı olabilir; agresif önceden yükleme yapma.

### 4.1 Gözlenen REST trafiği — 2026-11-09 (Linux, Chrome 155, web, yan hesap)

**Başlıklar — ✅ gözlendi** (`GET /channels/{id}/messages`, diğer `/api/v9` istekleri aynı küme):
`Authorization` (öneksiz token), `User-Agent`, `X-Super-Properties`, `X-Discord-Locale: tr`
(sadece dil), `X-Discord-Timezone: Europe/Istanbul` (IANA), `X-Debug-Options: bugReporterEnabled`,
`X-Installation-ID: <snowflake>.<imza>` (IDENTIFY'daki `installation_id` ile **aynı** değer → kalıcı,
kuruluma özel), `Referer: https://discord.com/channels/<guild>/<kanal>`,
`sec-ch-ua: "Google Chrome";v="155", "Chromium";v="155", "Not(A:Brand";v="24"`,
`sec-ch-ua-mobile: ?0`, `sec-ch-ua-platform: "Linux"`.

**`X-Super-Properties`** = base64(JSON). IDENTIFY `properties` ile aynı alanlar **şu farklarla**:
- `is_fast_connect`, `gateway_connect_reasons`, `installation_id` **yok**.
- `launch_signature` **var** (UUID biçimli; her açılışta farklı → istemci üretiyor olabilir — doğrulanmadı).
- `client_heartbeat_session_id` **var** (UUID; op 41'deki `session_id` ve bazı sorgu
  parametreleriyle aynı → istemci üretimi oturum kimliği).
- Sıra: `os, browser, device, device_model_name, system_locale, has_client_mods, browser_user_agent,
  browser_version, os_version, referrer, referring_domain, referrer_current, referring_domain_current,
  release_channel, client_build_number, client_event_source, client_launch_id, launch_signature,
  client_heartbeat_session_id, client_app_state`.

**Rate limit başlıkları:** CDP'nin gördüğü yanıt başlıklarında `x-ratelimit-*` **görülmedi** (200 yanıtlar).
CDP'nin bazı başlıkları göstermemesi mümkün — **doğrulanmadı**. 429 işleme yine de zorunlu.

**Kanal açılınca giden gateway mesajları — ✅ gözlendi**
- op 37 guild aboneliği. Küçük sunucu: `{"subscriptions":{"<guild>":{"typing":true,"activities":true,"threads":true}}}`.
  Büyük sunucu (~250 bin üye): ek olarak `"members":[],"member_updates":false,"channels":{},"thread_member_lists":[]`.
  Büyük sunucuda op 37 sonrası `GUILD_CREATE` geldi (tam guild nesnesi, `lazy: true`).
- op 43: `{"guild_id":"<guild>","fields":["status","voice_start_time"]}` → `CHANNEL_INFO`.
- op 8: `{"guild_id":["<guild>"],"user_ids":[<görünen yazarlar>],"presences":false}` → `GUILD_MEMBERS_CHUNK`
  (üye: `nick`, `roles`, `avatar`, `user`…). Mesaj yazarlarının takma ad/rol rengi için.
- Abonelikten sonra `MESSAGE_CREATE/UPDATE/DELETE`, `TYPING_START`, `PRESENCE_UPDATE` geldi.
  **Abonelik olmadan** büyük sunucuda MESSAGE_CREATE gelip gelmediği **doğrulanmadı** → resmi istemci
  gibi kanal açılınca op 37 gönderilecek.
- `MESSAGE_CREATE.d`: `id, channel_id, guild_id, author{id,username,global_name,avatar,…}, member{nick,roles,…},
  content, timestamp, edited_timestamp, type, flags, mentions, mention_roles, mention_everyone,
  attachments, embeds, components, pinned, tts, nonce, channel_type`. `MESSAGE_DELETE.d`: `id, channel_id, guild_id`.

### 4.2 Aşama 3 yazma istekleri — ⚠️ hepsi doğrulanmadı (resmi istemciden kayıt bekliyor)

Biçimler Discord'un herkese açık API dokümanı ve topluluk bilgisinden; canlı kullanımdan önce
resmi web istemcisinin trafiğiyle karşılaştırılacak (kullanıcı kararı: testler en sonda).
- `POST /channels/{c}/messages`: `{"mobile_network_type":"unknown","content","nonce":"<snowflake>","tts":false,"flags":0}`;
  yanıtta `message_reference{guild_id?,channel_id,message_id}` + `allowed_mentions`; ekli mesajda `attachments[]`, `type:0`, `sticker_ids:[]`.
- `PATCH /channels/{c}/messages/{m}` `{"content"}` · `DELETE /channels/{c}/messages/{m}`.
- `PUT /channels/{c}/messages/{m}/reactions/{emoji}/@me?location=Message&type=0`;
  `DELETE …/reactions/{emoji}/0/@me?location=Message&burst=false`.
- `POST /channels/{c}/typing` (boş gövde), ~8 sn'de bir.
- Yükleme: `POST /channels/{c}/attachments {"files":[{file_size,filename,id,is_clip}]}` → `upload_url`'ye
  token'sız PUT → mesajda `uploaded_filename`. Yükleme sınırı 10 MB varsayıldı.
- Rate limit: `X-RateLimit-*` başlıkları varsa bucket yönetimi, yoksa sadece 429.

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
