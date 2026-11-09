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

### 4.3 Aşama 4 — okunma durumu ve bildirim ayarları

- ✅ READY yapısı (2026-11-09 kaydından): `read_state.entries[]` = `{id (kanal), last_message_id
  (string, bazen sayı veya null), mention_count, flags, last_pin_timestamp, last_viewed?}`;
  `read_state_type` dolu kayıtlar kanal değil (atlanıyor). `user_guild_settings.entries[]` =
  `{guild_id (null = DM'ler), muted, mute_config{end_time}?, message_notifications (0 tümü,
  1 bahsetmeler, 2 hiçbiri), suppress_everyone, suppress_roles, channel_overrides[{channel_id,
  muted, mute_config, message_notifications (3 = üst ayar)}]}`. `relationships[]` = `{id, type
  (1 arkadaş, 2 engelli, 3 gelen, 4 giden), nickname}`.
- ⚠️ doğrulanmadı: `POST /channels/{c}/messages/{m}/ack` gövde `{"token":null}` (resmi istemci
  `last_viewed` gönderiyor olabilir); `MESSAGE_ACK` olayı alanları `{channel_id, message_id,
  mention_count?}`; ack sıklığı (biz kanal başına en fazla 3 sn'de bir).
- ⚠️ Okunma kaydı olmayan kanal "okunmuş" sayılıyor; resmi istemcinin davranışı doğrulanmadı.
- ⚠️ Kullanıcı bildirim ayarı olmayan sunucuda sunucunun varsayılan bildirim seviyesi yerine
  "sadece bahsetmeler" varsayılıyor.

### 4.4 Aşama 7 — thread'ler ve forum kanalları (⚠️ kullanıcı-hesabı davranışı doğrulanmadı)

Thread tipleri (belgeli): 10 duyuru thread'i, 11 herkese açık thread, 12 özel thread; 15 forum,
16 medya kanalı. Thread nesnesi kanal nesnesidir: `parent_id` (üst kanal), `owner_id`,
`thread_metadata{archived, locked, archive_timestamp, auto_archive_duration}`, `message_count`,
`member_count`, `member` (yalnızca benim üyeliğim varsa), `applied_tags`; forumda `available_tags[]`.
Thread'in kendi izin overwrite'ı yoktur; izinler üst kanaldan hesaplanır, yazmak için
`SEND_MESSAGES_IN_THREADS` (1<<38) gerekir, kilitli thread'e `MANAGE_THREADS` (1<<34) olmadan yazılamaz.

- ⚠️ doğrulanmadı: kullanıcı hesabı READY'sinde guild nesnesinin `threads[]` alanı (bot biçimi
  GUILD_CREATE'teki `threads` ile aynı varsayıldı). Alan yoksa/boşsa thread listesi olayların ve
  REST'in doldurduğu kadardır; op 37'deki `threads: true` aboneliği zaten gönderiliyor.
- ⚠️ doğrulanmadı: `THREAD_CREATE`, `THREAD_UPDATE` (tam thread nesnesi, `member` yok),
  `THREAD_DELETE` (`{id, guild_id, parent_id, type}`), `THREAD_LIST_SYNC`
  (`{guild_id, channel_ids?, threads[], members[]}`; `channel_ids` yoksa tüm guild) kullanıcı
  hesabında bot belgelerindeki biçimle geliyor varsayıldı. `THREAD_MEMBERS_UPDATE`,
  `THREAD_MEMBER_UPDATE` ve `THREAD_MEMBER_LIST_UPDATE` yok sayılır.
- ⚠️ doğrulanmadı: forumun aktif gönderileri
  `GET /channels/{forum}/threads/search?archived=false&sort_by=last_message_time&sort_order=desc&limit=25`
  (belgesiz kullanıcı-istemcisi uç noktası; parametre adları/değerleri topluluk bilgisi). Yanıt
  `{threads[], members[], has_more, total_results, first_messages[]}`; yalnızca `threads` ve
  `has_more` okunur. Sayfalama (`offset`) yok: ilk 25.
- Belgeli ama kullanıcı hesabında denenmedi: `GET /channels/{c}/threads/archived/public?limit=25`
  (`{threads[], members[], has_more}`); `before` sayfalaması uygulanmadı. Bot belgesi
  `READ_MESSAGE_HISTORY` ister.
- ⚠️ Özel thread'ler: sunucu bunları yalnızca üyelere / `MANAGE_THREADS` sahiplerine gönderir
  varsayıldığından gelen her thread gösterilir; üyelik (`member`) ile ayrıca süzülmez.
- ⚠️ Thread okunma durumu `read_state.entries[]` içinde thread id'siyle geliyor varsayıldı;
  sessize alma/bildirim düzeyi thread'in kendi `channel_overrides` kaydı yoksa üst kanalınkinden
  alınır (resmi davranış doğrulanmadı).
- Thread'e mesaj: normal `POST /channels/{thread_id}/messages` (Referer: `/channels/{guild}/{thread}`).
  Arşivli (kilitsiz) thread'e yazmak onu sunucuda açar, istemci ayrıca bir şey göndermez (doğrulanmadı).
- Kapsam dışı: thread/forum gönderisi **oluşturma**, thread'e katılma/ayrılma, etiket filtreleme.

### 4.5 Aşama 7 — arama, sabitlenenler ve mesaja atlama (⚠️ arama uç noktaları doğrulanmadı)

Hepsi **kullanıcı eylemiyle** tetiklenir: `:search` / `:pins` / sayfa tuşu (`n`/`p`) başına tek istek;
arka plan, otomatik yeniden deneme ya da toplu arama yok. Rate limit rotaları mevcut
bucket mantığıyla yönetilir (`GET /guilds/:id/messages/search`).

- ⚠️ doğrulanmadı: sunucu araması `GET /guilds/{guild_id}/messages/search?content=<q>&offset=<n>`;
  DM araması `GET /channels/{channel_id}/messages/search?content=<q>&offset=<n>` (belgesiz
  kullanıcı-istemcisi uç noktaları; adlar topluluk bilgisi). `content` UTF-8 yüzde-kodlu
  (`çay şeker` → `%C3%A7ay%20%C5%9Feker`). Sayfa boyutu sunucuda sabit varsayıldı (25); `offset`
  25'er artırılır, `limit` gönderilmez.
- ⚠️ doğrulanmadı: isteğe bağlı süzgeç parametreleri — `author_id=<kullanıcı>` (`from:`),
  `channel_id=<kanal>` (`in:`; yalnızca sunucu aramasında), `has=link|file|image|embed`
  (`has:`; birden çoksa `has=` tekrarlanır). Sıralama parametresi gönderilmez.
- ⚠️ doğrulanmadı: yanıt `{"total_results": N, "messages": [[Message, ...], ...]}`. Her iç dizi
  bir isabettir: `"hit": true` alanlı eleman isabet, diğerleri bağlam mesajlarıdır; `hit` yoksa
  ilk eleman alınır. Bağlam mesajları atılır, ayrıştırılamayan gruplar atlanır. `total_results`
  yoksa isabet sayısı kullanılır.
- ⚠️ doğrulanmadı: dizin hazır değilse sunucu **202** döner, gövdede `retry_after` (saniye)
  olur. İstemci yeniden denemez; sonuç ekranında "Arama dizini hazırlanıyor, birazdan tekrar
  dene" gösterir, kullanıcı komutu kendisi tekrarlar. 429 normal rate limit yoluyla işlenir
  (en çok 3 bekleme + yeniden deneme, bkz. §4).
- Belgeli: sabitlenenler `GET /channels/{id}/pins` → `[Message]` (sunucu en son sabitleneni
  başa koyar). ⚠️ doğrulanmadı: daha yeni sayfalı uç nokta (`GET /channels/{id}/messages/pins`,
  `{"items":[{"pinned_at","message"}],"has_more"}`) kullanılmıyor; ancak yanıt bu nesne biçiminde
  gelirse ayrıştırıcı `items[].message` alanlarını da kabul eder. Sayfalama yok: gelen liste.
- Belgeli: bir mesaja atlamak için `GET /channels/{id}/messages?around=<mesaj>&limit=50`
  (en yeniden en eskiye, hedefin iki yanında yaklaşık yarı yarıya). Sonuç store'da mevcut
  pencereyle **örtüşüyorsa** birleştirilir (kopyalar tek kalır), örtüşmüyorsa pencere
  değiştirilir; kanalın bilinen en yeni mesajına ulaşmayan pencere "ayrık" sayılır (gateway'den
  gelen yeni mesajlar eklenmez, kanaldan çıkınca/`G` ile atılır, yazma kapalı). Hedef yanıtta
  yoksa (silinmiş) kullanıcıya söylenir.
- Arama sonuç mesajlarındaki `member` / `guild_id` alanlarının eksik olabileceği varsayıldı;
  yazar adı store'daki üye bilgisinden, yoksa mesajın `author`'ından gösterilir. Sonuçlar
  store'a yazılmaz (yalnızca atlanan mesajın `around` penceresi yazılır).

## 5. Davranış kuralları (ban riskini azaltmak için)

- Resmi istemcinin **yapmadığı** şeyi yapma: toplu geçmiş indirme, otomatik yanıt, çok hızlı
  kanal gezme, gizli API uçları.
- Görünür olmayan kanallar için mesaj çekme / abonelik yapma.
- Yazıyor/okundu/presence davranışı resmi istemciye benzesin (ayarlanabilir: "okundu gönderme" kapatılabilir).

## 6. CDN ve medya

- Avatar/emoji/ek URL'leri `cdn.discordapp.com` / `media.discordapp.net`.
- Boyut parametresiyle (`?size=64`, `?width=&height=`) küçük sürüm iste → bant genişliği tasarrufu.
- Ek URL'leri imzalı ve süreli olabilir; süresi dolanlar yeniden çözülmeli (refresh uç noktası).

### 6.1 Aşama 5 uygulaması

- **İzin listesi** (`digcord-media::ALLOWED_HOSTS`): `cdn.discordapp.com`, `media.discordapp.net`,
  `images-ext-1.discordapp.net`, `images-ext-2.discordapp.net`; sadece `https`, varsayılan port.
  Yönlendirmeler de listeye tabi. Embed'lerin dış `url`'sine **gidilmez**, sadece `proxy_url`'ye.
- CDN isteklerinde `Authorization` yok; User-Agent `ClientProperties`'ten (tarayıcıyla aynı).
  Diğer tarayıcı başlıkları (`sec-ch-ua`, `Referer`...) gönderilmiyor. ⚠️ doğrulanmadı
  (resmi istemcinin CDN istek başlıkları kayıtla karşılaştırılmalı).
- Özel emoji: `https://cdn.discordapp.com/emojis/{id}.png?size=48` (belgelenmiş "Image
  Formatting"; hareketli emojide PNG ilk kareyi verir).
- Ek ve embed önizlemesi: `proxy_url` + `width=W&height=H` (hedef piksel boyutu). Vekilin
  küçültülmüş resim döndürdüğü varsayılıyor. ⚠️ doğrulanmadı (resmi istemcinin önizleme
  isteğiyle karşılaştırılmalı; resmi istemci ayrıca `format=webp` vb. ekleyebilir).
- İmzalı adresin süresi dolarsa (CDN 403/404/410 ve URL'de `ex=`): refresh uç noktası **kullanılmıyor**;
  mesaj bir kez `GET /channels/{id}/messages?before={id+1}&limit=1` ile yeniden çekilip yeni
  adres kullanılıyor (resmi istemcinin geçmiş isteğiyle aynı uç nokta). ⚠️ resmi istemcinin bu
  durumda ne yaptığı doğrulanmadı.
- Disk önbelleği `~/.cache/digcord/media` (0700); dosya adı imza parametreleri (`ex`, `is`, `hm`)
  atılmış URL'nin özeti. Açılışta 512 MB'ı aşan en eski dosyalar silinir. Tek yanıt en fazla 16 MB.

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

### 7.1 Aşama 6 uygulaması (hepsi belgelere + açık kaynak referanslara göre; ⚠️ resmi istemci trafiğiyle karşılaştırılmadı)

Kaynaklar: Discord geliştirici belgeleri (voice connections, opcodes), DAVE beyaz kağıdı
v1.1.4 (CC BY-NC-SA — kopyalanmadı, okundu), libdave (MIT), davey (MIT, sadece okundu;
bağımlılık değil), discord.js voice (Apache-2.0). Ayrıntılı byte düzenleri kodda ve testlerde.

- **op 4** (ana gateway): `{guild_id, channel_id, self_mute, self_deaf, self_video:false}`.
  Resmi istemcinin ek alanları (`flags` vb.) ⚠️ doğrulanmadı.
- **DM araması:** op 4'te `guild_id: null`; ses gateway'inde `server_id` = kanal id'si ⚠️
  doğrulanmadı. Aramayı başlatınca karşı tarafı çaldırma (ring) uç noktası **kullanılmıyor**;
  `:call` sadece katılır.
- **Ses gateway'i:** `wss://{endpoint}/?v=8`, Identify'da `max_dave_protocol_version: 1`,
  heartbeat `{t, seq_ack}`, resume `{server_id, session_id, token, seq_ack}`. Bağlantı
  başlıkları: `Origin: https://discord.com`, User-Agent `ClientProperties`'ten ⚠️ doğrulanmadı.
- **UDP:** 74 baytlık IP keşfi; 5 sn'de bir 8 baytlık canlı tutma (discord.js davranışı,
  belgelerde yok ⚠️); RTP PT 0x78; taşıma nonce'u 4 bayt sayaç (0'dan başlıyor; discord.js 1'den
  — alıcı sayıyı yorumlamadığı için fark etmez).
- **Şifreleme kipi:** `aead_aes256_gcm_rtpsize` tercih, yoksa `aead_xchacha20_poly1305_rtpsize`.
- **DAVE:** ciphersuite 2 (P-256/AES-128-GCM/SHA-256), kimlik = kullanıcı id'si 8 bayt BE,
  grup id = kanal id'si 8 bayt BE, exporter etiketi `Discord Secure Frames v0` + kullanıcı id'si
  8 bayt **LE**, 16 bayt. op 26 çıplak KeyPackage, op 28 commit + çıplak Welcome (beyaz kağıt
  "MLSMessage" diyor; libdave ve davey çıplak gönderiyor — uygulamalar izlendi ⚠️).
  Kare: AES-128-GCM, 8 baytlık etiket, nonce LE bayt 8..12, ULEB128 nonce, `0xFAFA`.
  Opus sessizlik karesi (`F8 FF FE`) DAVE'siz gider.
- **Geçişler:** op 21/22/23/24 ve 31 kurtarma (geçersiz commit/welcome → op 31 + yeni anahtar
  paketi). Eski epoch anahtarları 10 sn geçerli; tekrar penceresi 1000.
- **Kapanış:** 4006/4009 → ana gateway'den yeniden katılma (bir kez); 4014/4017/4021/4022 ve
  diğer resume edilemezler → bağlantı biter; diğerleri → resume (en fazla 5).
- **Test durumu:** çevrimdışı birim/entegrasyon testleri (iki istemcinin tam DAVE akışı, sahte
  sunucuyla WS+UDP). **Canlı bağlantı hiç denenmedi**; kabul testi bekliyor.

## Kaynaklar (doğrulanacak)

- Discord resmi geliştirici dokümanları (bot API'si; kullanıcı API'sine çoğu benzer).
- Topluluk kullanıcı API'si dokümanları (ör. "discord-userdoccers").
- `discord/dave-protocol` ve `discord/libdave` depoları.
- Mevcut TUI istemcileri (discordo — Go, Endcord — Python) **sadece davranış referansı** için.
