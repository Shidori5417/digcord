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

## Aşama 1 — Gateway + giriş ⏳ (kod tamam, canlı kabul testi bekliyor)
- [x] Token giriş ekranı + keyring'e kaydetme/okuma.
- [x] Gateway: zlib-stream, Hello, Identify, heartbeat (op 40), READY ayrıştırma.
- [x] Resume + yeniden bağlanma + close kodu tablosu (sahte yerel gateway ile uçtan uca test).
- [x] Gerçek gateway verisinden kaydedilmiş (anonimleştirilmiş) örneklerle ayrıştırma testleri.

**Kabul:** Giriş yapılıyor, READY'den sunucu ve kanal sayısı durum çubuğunda görünüyor;
ağ kesilip gelince otomatik resume oluyor.
- [ ] Canlı test (yan hesap) — kullanıcı isteğiyle ertelendi (2026-11-09). Öncesinde
  PROTOCOL_NOTES §3.1'deki "bilinen farklar" için karar verilecek.

## Aşama 2 — Okuma ⏳ (kod tamam, canlı kabul testi bekliyor)
- [x] Store, sunucu şeridi, kanal ağacı (kategoriler, izinlere göre görünür kanallar).
- [x] Mesaj geçmişi çekme, yukarı kaydırınca sayfalama, canlı `MESSAGE_CREATE/UPDATE/DELETE`.
- [x] Markdown alt kümesi, mention çözümleme, gruplanmış mesajlar.
- [x] Sanal kaydırma; 10 000 mesajlık kanalda akıcı kaydırma.

**Kabul:** Herhangi bir kanal açılıp geçmiş okunabiliyor, yeni mesajlar anında düşüyor,
ölçülen kare süresi < 4 ms (release, 200×60 terminal).
- [x] Kare süresi: ~0,54 ms (release, 200×60, 10 000 mesaj, `TestBackend`; terminal G/Ç hariç) —
  `cargo test -p digcord-tui --release -- --ignored --nocapture frame_time`.
- [ ] Canlı test (yan hesap): kanal açma, geçmiş, canlı mesajlar — Aşama 1 canlı testiyle birlikte.

## Aşama 3 — Yazma ⏳ (kod tamam, canlı kabul testi ve protokol kaydı bekliyor)
- [x] Mesaj gönderme (nonce + iyimser güncelleme), yanıt, düzenleme, silme, tepki.
- [x] Çok satırlı giriş, yazıyor bildirimi, emoji/mention otomatik tamamlama.
- [x] Dosya ekleme (`:upload <yol> [mesaj]`).
- [x] Rate limit bucket yönetimi + testleri.

**Kabul:** Resmi istemciyle yan yana açıkken tüm eylemler iki tarafta tutarlı görünüyor.
- [ ] Önce yazma isteklerinin resmi istemci trafiğiyle karşılaştırılması (PROTOCOL_NOTES §4.2), sonra canlı test.

## Aşama 4 — DM, okunmamış, bildirim ⏳ (kod tamam, canlı kabul testi bekliyor)
- [x] DM ve grup DM listesi, arkadaş listesi (salt okunur başlangıç).
- [x] read_state, okundu gönderme (ack), okunmamış/mention sayaçları.
- [x] Masaüstü bildirimleri, sessize alma ayarlarına uyma.
- [x] `Ctrl+K` hızlı geçiş (+ `Alt+↑/↓` okunmamış kanallar).
- [x] `config.toml` (davranış, bildirim, istemci build numarası).

**Kabul:** Başka hesaptan gelen DM ve mention'lar bildirim + sayaç olarak doğru görünüyor;
kanal açılınca sayaçlar resmi istemcide de sıfırlanıyor.
- [ ] Canlı test + ack isteğinin resmi istemciyle karşılaştırılması (PROTOCOL_NOTES §4.3).

## Aşama 5 — Medya ⏳ (kod tamam, canlı kabul testi bekliyor)
- [x] Terminal grafik algılama, resim ekleri ve özel emoji gösterimi, LRU + disk önbelleği.
  (Özel emoji satır içinde metin; sadece-emoji mesajlarda büyük resim — kullanıcı onayıyla.)
- [x] Tam ekran resim görüntüleyici, `o` ile harici açma.
- [x] Embed'ler (başlık, açıklama, renkli kenar, küçük resim).

**Kabul:** Kitty/WezTerm/Konsole'da resimler görünüyor, Alacritty'de yarım blok yedeği çalışıyor,
resimli kanalda kaydırma takılmıyor.
- [x] Çevrimdışı: yarım blok yedeği TestBackend'de çiziliyor; 10k mesajda kare ~0,7 ms.
- [ ] Canlı test: Kitty, WezTerm, Konsole, Alacritty; CDN isteklerinin resmi istemciyle
  karşılaştırılması (PROTOCOL_NOTES §6.1).

## Aşama 6 — Ses (DAVE ile) ⏳ (kod tamam, canlı kabul testi bekliyor)
Alt adımlar sırayla:
1. [x] Ses gateway'i + UDP + IP keşfi + transport şifreleme (birim testleri).
2. [x] **DAVE**: MLS el sıkışması ve kare şifreleme — test vektörleri (davey ratchet vektörü +
   bağımsız hesaplanmış çerçeve vektörleri; libdave'de hex vektör yok) ve iki istemcili tam akış testi.
3. [x] Opus + cpal: dinleme (jitter buffer, karıştırıcı).
4. [x] Konuşma: VAD/PTT (PTT kitty klavye protokolüyle), mikrofon/kulaklık susturma.
5. [x] Kullanıcı başına ses seviyesi (`:vol`), konuşan göstergesi, jitter buffer ayarı.

**Kabul:** Sunucu ses kanalında ve DM aramasında karşılıklı konuşulabiliyor, bağlantı 4017 ile
reddedilmiyor, 30 dk kesintisiz çalışıyor, ses thread'i CPU'nun < %3'ünü kullanıyor.
- [x] Çevrimdışı: ses yolu kare başına ~175 µs (1 gönderen + 3 dinlenen, tek çekirdeğin %0,9'u).
- [ ] Canlı test: sunucu ses kanalı, DM araması, 4017 yok, 30 dk; protokol karşılaştırması (PROTOCOL_NOTES §7.1).

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
