# Yol Haritası

Her aşama bir öncekinin üzerine kurulur. Bir aşama, **kabul kriterlerinin tamamı** sağlanmadan bitmiş sayılmaz.
Testler yan hesapta, kendi açtığın test sunucusunda yapılır.

## Aşama 0 — İskelet ✅ (v0.1.0)
- [x] Git deposu + **private** GitHub deposu (bkz. `GIT_WORKFLOW.md`), dokümanlar ilk commit olarak push'lanmış.
- [x] Cargo workspace, crate'ler boş ama derleniyor; `cargo clippy --all-targets -- -D warnings` temiz.
- [x] Tüm crate'lerde `license-file = "LICENSE"` ve `publish = false`.
- [x] tracing → dosyaya log (`~/.local/state/digcord/digcord.log`), token maskeleme.
- [x] ratatui iskeleti: boş üç panel, `q` ile çıkış, panic'te terminal geri yükleniyor.
- [x] CI: fmt + clippy + test (GitHub Actions).

**Kabul:** `cargo run` boş arayüzü açıyor, çıkış ve panic sonrası terminal bozulmuyor;
GitHub'da depo private görünüyor, CI yeşil, `v0.1.0` etiketi atılmış.

## Aşama 1 — Gateway + giriş
- Token giriş ekranı + keyring'e kaydetme/okuma.
- Gateway: zlib-stream, Hello, Identify, heartbeat, READY ayrıştırma.
- Resume + yeniden bağlanma + close kodu tablosu.
- Gerçek gateway verisinden kaydedilmiş (anonimleştirilmiş) örneklerle ayrıştırma testleri.

**Kabul:** Giriş yapılıyor, READY'den sunucu ve kanal sayısı durum çubuğunda görünüyor;
ağ kesilip gelince otomatik resume oluyor.

## Aşama 2 — Okuma
- Store (actor), sunucu şeridi, kanal ağacı (kategoriler, izinlere göre görünür kanallar).
- Mesaj geçmişi çekme, yukarı kaydırınca sayfalama, canlı `MESSAGE_CREATE/UPDATE/DELETE`.
- Markdown alt kümesi, mention çözümleme, gruplanmış mesajlar.
- Sanal kaydırma; 10 000 mesajlık kanalda akıcı kaydırma.

**Kabul:** Herhangi bir kanal açılıp geçmiş okunabiliyor, yeni mesajlar anında düşüyor,
ölçülen kare süresi < 4 ms (release, 200×60 terminal).

## Aşama 3 — Yazma
- Mesaj gönderme (nonce + iyimser güncelleme), yanıt, düzenleme, silme, tepki.
- Çok satırlı giriş, yazıyor bildirimi, emoji/mention otomatik tamamlama.
- Dosya ekleme (yol ile veya `:upload`).
- Rate limit bucket yönetimi + testleri.

**Kabul:** Resmi istemciyle yan yana açıkken tüm eylemler iki tarafta tutarlı görünüyor.

## Aşama 4 — DM, okunmamış, bildirim
- DM ve grup DM listesi, arkadaş listesi (salt okunur başlangıç).
- read_state, okundu gönderme (ack), okunmamış/mention sayaçları.
- Masaüstü bildirimleri, sessize alma ayarlarına uyma.
- `Ctrl+K` hızlı geçiş.

**Kabul:** Başka hesaptan gelen DM ve mention'lar bildirim + sayaç olarak doğru görünüyor;
kanal açılınca sayaçlar resmi istemcide de sıfırlanıyor.

## Aşama 5 — Medya
- Terminal grafik algılama, resim ekleri ve özel emoji gösterimi, LRU + disk önbelleği.
- Tam ekran resim görüntüleyici, `o` ile harici açma.
- Embed'ler (başlık, açıklama, renkli kenar, küçük resim).

**Kabul:** Kitty/WezTerm/Konsole'da resimler görünüyor, Alacritty'de yarım blok yedeği çalışıyor,
resimli kanalda kaydırma takılmıyor.

## Aşama 6 — Ses (DAVE ile)
Alt adımlar sırayla:
1. Ses gateway'i + UDP + IP keşfi + transport şifreleme (birim testleri).
2. **DAVE**: MLS el sıkışması ve kare şifreleme — libdave/beyaz kağıt test vektörleriyle birim testleri.
3. Opus + cpal: sadece dinleme.
4. Konuşma: VAD/PTT, mikrofon/kulaklık susturma.
5. Kullanıcı başına ses seviyesi, konuşan göstergesi, jitter buffer ayarı.

**Kabul:** Sunucu ses kanalında ve DM aramasında karşılıklı konuşulabiliyor, bağlantı 4017 ile
reddedilmiyor, 30 dk kesintisiz çalışıyor, ses thread'i CPU'nun < %3'ünü kullanıyor.

## Aşama 7 — Cila
- Temalar, tam kısayol özelleştirme, ayarlar ekranı.
- Thread'ler ve forum kanalları.
- Arama (`/search` uç noktası), sabitlenen mesajlar.
- Performans profili: açılış süresi, bellek (hedef: büyük hesapta < 150 MB).
- Paketleme: AUR paketi (`digcord-git`).

## Sonraki opsiyonel aşamalar (ana yapının dışında)
Bunlar proje bittikten sonra istenirse eklenecek; ana mimari bunları engellemeyecek şekilde
(ör. `digcord-voice` içinde video/stream için genişletilebilir arayüzler) tasarlanır ama **uygulanmaz**:
- Görüntülü arama (kamera)
- Ekran paylaşımı / Go Live izleme ve yayınlama
- Etkinlikler (Activities)

## Kapsam dışı (bilinçli olarak)
- Self-bot / otomasyon / toplu mesaj özellikleri
- Captcha atlatma
- Nitro özelliklerini taklit etme
