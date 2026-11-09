# TUI Tasarımı

## Ana düzen

```
┌──┬──────────────────┬─────────────────────────────────────────┬────────────────┐
│DM│ ▾ BİLGİ           │ #genel  — Sunucu konusu                 │ ÇEVRİMİÇİ — 12 │
│──│   # duyurular    │─────────────────────────────────────────│ ● Ahmet        │
│S1│   # genel     3  │ Ahmet  14:02                            │ ● Ayşe         │
│S2│ ▾ SES            │   selam herkese                         │ ◐ Can          │
│S3│   🔊 Sohbet       │ Ayşe  14:03                             │                │
│  │     ● Can        │   ↳ Ahmet: selam herkese                │ ÇEVRİMDIŞI — 30│
│  │                  │   naber 👋                               │ ○ ...          │
│  │                  │   [resim: ekran.png 1280×720]           │                │
│  │                  │                                         │                │
│  │                  │─────────────────────────────────────────│                │
│  │ 🔊 Sohbet  🎤 🎧   │ > mesaj yaz...                          │                │
└──┴──────────────────┴─────────────────────────────────────────┴────────────────┘
 [NORMAL]  Digcord · bağlı · 42 ms                              Ayşe yazıyor...
```

- **Sunucu şeridi** (en sol, dar): DM + sunucu kısaltmaları/ikonları, okunmamış noktası, mention sayısı.
- **Kanal ağacı:** kategoriler katlanabilir, okunmamış kalın, mention sayısı sağda. Altta ses durumu çubuğu.
- **Mesaj görünümü:** gruplanmış mesajlar (aynı yazar, < 7 dk), yanıt önizlemesi, tepkiler, ekler, embed'ler.
- **Üye listesi:** isteğe bağlı (`Ctrl+M` aç/kapat), dar terminalde otomatik gizlenir.
- **Durum çubuğu:** mod, bağlantı durumu/gecikme, yazıyor bilgisi, hatalar.

Duyarlı düzen: < 100 sütun → üye listesi kapanır; < 70 sütun → kanal ağacı açılır pencereye döner.

## Modlar (vim benzeri, opsiyonel)

| Mod | Açıklama |
|---|---|
| NORMAL | Gezinme, mesaj seçme |
| INSERT | Mesaj yazma |
| SELECT | Mesaj üzerinde eylem (yanıtla, düzenle, sil, tepki, kopyala, eki aç) |
| COMMAND | `:` komutları (`:join`, `:quit`, `:theme`) |

Vim istemeyenler için config'de `keymap = "simple"` (her zaman yazma modu, ok tuşlarıyla gezinme).

## Varsayılan kısayollar

| Tuş | Eylem |
|---|---|
| `Ctrl+K` | Hızlı geçiş (kanal/sunucu/DM bulanık arama) |
| `Tab` / `Shift+Tab` | Paneller arası odak |
| `j` / `k` | Aşağı / yukarı |
| `i` | Yazma moduna geç |
| `v` (veya mesaj panelinde `Enter`) | SELECT: mesaj seç (`j`/`k`) |
| `:` | COMMAND: `:upload <yol> [mesaj]`, `:q` |
| `Tab` (INSERT) | Otomatik tamamlamayı kabul et (`@kişi`, `#kanal`, `:emoji`) |
| `Esc` | NORMAL moda dön / pencereyi kapat |
| `Enter` (INSERT) | Gönder; `Shift+Enter`/`Alt+Enter` yeni satır |
| `r` | Seçili mesaja yanıt |
| `e` | Kendi mesajını düzenle (INSERT'te boşken `↑` de son mesajı düzenler) |
| `d d` | Sil (onay ister) |
| `+` | Tepki ekle/kaldır (emoji veya `:kısa_ad:` istemi) |
| `o` | Resimli mesajda tam ekran görüntüleyici; orada `o` harici açar (xdg-open), `←/→` (`h/l`) resimler arası, `Esc` kapatır. Resimsiz mesajda ilk bağlantıyı/eki harici açar |
| `y` | Mesaj metnini kopyala |
| `Ctrl+M` | Üye listesi aç/kapat |
| `Ctrl+Shift+M` / `Ctrl+Shift+D` | Mikrofon / kulaklık sessize al (kitty klavye protokolü gerekir; yoksa `:mute` / `:deaf`) |
| `Enter` (ses kanalında) | Ses kanalına katıl |
| `Ctrl+Space` (basılı tut) | Bas-konuş (`voice.mode = "ptt"`; kitty klavye protokolü gerekir) |
| `:leave` / `:mute` / `:deaf` | Ses kanalından ayrıl / mikrofon / kulaklık |
| `:devices` | Kullanılabilir ses giriş/çıkış aygıtlarını listele (durum çubuğuna tek satır; tam liste günlükte) |
| `:call` | Açık DM'de aramaya katıl (çaldırmaz) |
| `:vol <ad> <0-200>` | Ses kanalındaki kişinin ses seviyesi (yüzde) |
| `Alt+↑/↓` | Okunmamış önceki/sonraki kanal (sessizdekiler atlanır) |
| `q` / `:q` | Çıkış |

Normal/SELECT modu ve genel kısayollar `config.toml` `[keys]` tablosundan değiştirilebilir (aşağıda).
`Ctrl+C` (çıkış), INSERT/COMMAND metin düzenleme tuşları, hızlı geçiş/resim görüntüleyici/silme onayı
pencerelerinin içindeki tuşlar ve ayarlar ekranının tuşları sabittir. Alt çubuktaki ipucu metinleri
varsayılan tuşları gösterir. `Ctrl+M` (üye listesi aç/kapat) henüz uygulanmadı, bu yüzden atanabilir
eylemlerde yok.

## Mesaj işleme

- Markdown alt kümesi: **kalın**, *italik*, __altı çizili__, ~~üstü çizili~~, `kod`, kod blokları
  (dil adına göre sözdizimi renklendirme — `syntect` opsiyonel), `> alıntı`, `||spoiler||` (gizli, `o` ile aç),
  başlıklar, listeler.
- Mention'lar (`<@id>`, `<#id>`, `<@&rol>`) isimle ve renkle; kendine mention vurgulu arka plan.
- Özel emoji: satır içinde `:isim:` (terminal hücre yapısı yüzünden satır içi resim yok);
  sadece emojiden oluşan mesajlarda (en fazla 27) emojiler büyük resim olarak yan yana.
- Unicode emoji genişliği `unicode-width` ile; hizalama bozulmalarına dikkat.
- Zaman damgaları `<t:...:R>` biçimleri yerelleştirilmiş gösterilir.

## Resim gösterimi

- Algılama açılışta bir kez (`ratatui-image` picker; terminal sorgusu): Kitty → iTerm2 → Sixel →
  yarım blok (`▀`) yedeği. WezTerm/Konsole'da kitty ve sixel kapalı (iTerm2 kullanılır).
- Mesaj içinde önizleme: `thumbnail` en fazla 48 sütun, `full` mesaj alanı genişliği; ikisinde de
  en fazla `image_max_rows` satır. `off`: sadece `[ek: ad boyut]` satırı.
- Resim için sabit satır ayrılır (kaydırma hesabı resimden bağımsız). Resim sadece tamamen
  görünürken çizilir; kısmen görünürken yerinde `[resim]` yazar.
- İndirme, çözme, ölçekleme ve terminal protokolüne kodlama arka planda; UI sadece hazır resmi
  basar. Hazır resimler bellekte LRU (96), ham dosyalar diskte.
- Hareketli GIF/WebP: sadece ilk kare. Video ekleri: önizleme yok, `o` ile harici açılır.
- Embed: renkli sol kenar, sağlayıcı, yazar, başlık, açıklama (en fazla 12 satır), alanlar,
  alt bilgi ve (varsa) resim/küçük resim.

## Bildirimler

- Mention, DM ve "tüm mesajlar" bildirimi açık kanallarda masaüstü bildirimi (notify-rust → KDE).
- Sunucu/kanal sessize alma ayarlarına (READY'deki kullanıcı guild ayarları) uyulur.
- Terminal odaktaysa ve ilgili kanal açıksa bildirim gönderilmez.
- Opsiyonel terminal zili / pencere başlığında okunmamış sayısı.

## Ses arayüzü

- Kanal ağacında ses kanalındaki kullanıcılar; konuşan kişi yeşil, susturma simgeleri
  (`🎤✕`, `🎧✕`). ✅
- Durum çubuğu: bağlı kanal, aşama (katılınıyor/bağlanıyor/bağlı), "uçtan uca" (DAVE),
  mikrofon/kulaklık durumu. ✅ Gecikme gösterimi yok.
- Kullanıcı ses seviyesi `:vol` ile. ✅ Kaydırıcı ve yerel susturma yok (sonra).
- Ses aygıtı ve VAD eşiği config'ten: `voice.input_device` / `voice.output_device` (cpal aygıt
  adı, tam eşleşme; boş ya da `default` = sistem varsayılanı; bulunamazsa uyarılıp varsayılana
  dönülür). Adlar `:devices` ile görülür. `voice.vad_threshold` (RMS, 0.001–0.5, varsayılan
  0.012) `:settings`'ten `+`/`-` ile anında değişir. Aygıt değişikliği bir sonraki katılımda
  geçerli. Ayrı bir ses ayarları penceresi ve ayarlanabilir PTT tuşu yok: Ctrl+Space sabit. (sonra)

## Config

`~/.config/digcord/config.toml`

**Şu an desteklenenler:** `ui.theme`, `ui.show_member_list`, `ui.images`, `ui.image_max_rows`,
`voice.mode` (`"vad"`, `"ptt"`), `voice.jitter_ms`, `voice.vad_threshold` (0.001–0.5; aralık dışı
uyarıyla düzeltilir), `voice.input_device`, `voice.output_device`, `behavior.send_typing`,
`behavior.send_read_receipts`, `notifications.enabled`, `notifications.on` (`"mention"`, `"dm"`,
`"all"`), `client.build_number`, `client.chrome_major` ve `[keys]` tablosu. Diğer anahtarlar okunur
ama yok sayılır (açılışta uyarı); bozuk dosyada uygulama açılmaz ve hatayı gösterir.

```toml
[ui]
keymap = "vim"           # "vim" | "simple"
theme = "default"        # temalar ~/.config/digcord/themes/*.toml
timestamp_format = "%H:%M"
show_member_list = true
images = "thumbnail"     # "off" | "thumbnail" | "full"
image_max_rows = 10

[behavior]
send_typing = true
send_read_receipts = true
message_window = 200      # kanal başına bellekte tutulan mesaj

[notifications]
enabled = true
on = ["mention", "dm"]

[voice]
input_device = "default" # cpal aygıt adı (:devices); "default" = sistem varsayılanı
output_device = "default"
mode = "vad"             # "vad" | "ptt"
vad_threshold = 0.012    # ses algılama eşiği (RMS)
jitter_ms = 60           # çalmadan önce biriktirilen ses
ptt_key = "ctrl+space"

[client]
# İstemci kimliği profili — bkz. docs/PROTOCOL_NOTES.md
build_number = 0          # 0 = otomatik/varsayılan

[keys]
# "eylem" = "tuş" geçersiz kılmaları
quit = "ctrl+q"
toggle_mute = "ctrl+shift+m"
copy = ["y", "ctrl+c"]    # birden çok tuş için liste
```

Token config'de **tutulmaz** (keyring).

### Kısayol özelleştirme (`[keys]`)

`eylem = "tuş"` (ya da `eylem = ["tuş", ...]`). Bir eylem verilince **varsayılan tuşlarının hepsi**
yerine geçer. Tuş yazımı: tek karakter (`q`, `G`, `:`, `+`), adlar (`enter`, `esc`, `tab`, `space`,
`backspace`, `delete`, `insert`, `home`, `end`, `pageup`/`pgup`, `pagedown`/`pgdown`, `up`, `down`,
`left`, `right`, `f1`…`f24`) ve `ctrl+`, `alt+`, `shift+`, `super+` önekleri (`ctrl+k`, `alt+up`,
`shift+tab`, `ctrl+shift+m`). Büyük harf = Shift'li harf (`G` = `shift+g`).

Bilinmeyen eylem adı, ayrıştırılamayan tuş ve çakışmalar **uyarıdır** (açılışta durum çubuğunda ilk
uyarı gösterilir, hepsi log'a yazılır); açılış başarısız olmaz. Çakışmada (aynı tuş aynı kapsamda
ya da genel bir eylemle) son atama geçerlidir; eski sahibi o tuşu kaybeder. "Son", `[keys]` içinde
eylem adına göre alfabetik sıradır. Kapsamlar: *genel* (her modda), *NORMAL*, *SELECT*;
`move_down`/`move_up` NORMAL ve SELECT'te ortaktır.

| Kapsam | Eylem | Varsayılan | Açıklama |
|---|---|---|---|
| genel | `toggle_mute` | `ctrl+shift+m` | Mikrofonu sessize al |
| genel | `toggle_deaf` | `ctrl+shift+d` | Kulaklığı sessize al |
| genel | `push_to_talk` | `ctrl+space` | Bas-konuş (`voice.mode = "ptt"`; basma/bırakma olayları gerekir) |
| NORMAL | `quit` | `q` | Çıkış |
| NORMAL | `switcher` | `ctrl+k` | Hızlı geçiş |
| NORMAL | `next_focus` / `prev_focus` | `tab` / `shift+tab` | Panel odağı |
| NORMAL | `activate` | `enter`, `l` | Sunucu/kanal aç; mesaj panelinde `enter` SELECT'e girer |
| NORMAL | `insert_mode` | `i` | Yazma modu |
| NORMAL | `select_mode` | `v` | Mesaj seçme modu |
| NORMAL | `command_mode` | `:` | Komut modu |
| NORMAL | `page_down` / `page_up` | `pagedown`, `ctrl+d` / `pageup`, `ctrl+u` | Yarım sayfa kaydır |
| NORMAL | `scroll_bottom` | `G`, `end` | En alta in |
| NORMAL | `next_unread` / `prev_unread` | `alt+down` / `alt+up` | Okunmamış kanala atla |
| NORMAL+SELECT | `move_down` / `move_up` | `j`, `down` / `k`, `up` | Gezin / mesaj seçimini kaydır |
| SELECT | `back` | `esc` | NORMAL'e dön |
| SELECT | `reply` / `edit` | `r` / `e` | Yanıtla / düzenle |
| SELECT | `delete` | `d` | İki kez basılınca silme onayı (`d d`) |
| SELECT | `react` | `+` | Tepki istemi |
| SELECT | `copy` | `y` | Metni kopyala |
| SELECT | `open_media` | `o` | Resmi/bağlantıyı aç |
| SELECT | `retry` | `enter` | Başarısız mesajı yeniden gönder |

### Temalar

`ui.theme = "<ad>"` → `~/.config/digcord/themes/<ad>.toml` (`"default"` yerleşik, dosya aranmaz).
Dosya düz `yuva = "renk"` satırlarından oluşur. Renk: `"#rrggbb"`, ANSI adı (`red`, `darkgray`,
`lightblue`, `white`…), 0-255 indeks ya da `"reset"` (terminalin rengi). Eksik yuvalar yerleşik
temadan gelir. Bilinmeyen anahtar uyarı, geçersiz değer dosya adıyla birlikte hatadır; açılışta
bozuk tema uygulamayı durdurmaz (hata gösterilir, yerleşik tema kullanılır). Dosya okuma UI
thread'inde yapılmaz (açılışta ve `:theme` ile denetleyicide). Örnek: `themes/light.toml` (tüm yuvalar).

| Yuva | Varsayılan | Nerede |
|---|---|---|
| `background` | `reset` | Tüm ekran arka planı |
| `text` | `reset` | Varsayılan metin rengi |
| `border` | `reset` | Odakta olmayan panel çerçevesi |
| `border_focused` | `cyan` | Odaktaki panel, açılır pencereler, ayarlar |
| `author` | `cyan` | Mesaj yazarı (kalın) |
| `timestamp` | `reset` (soluk) | Saatler, `<t:…>` zaman damgaları |
| `dim` | `reset` (soluk) | İkincil metin, ipuçları, yanıt önizlemesi, ekler |
| `quote` | `reset` (soluk) | Alıntı (`>`) işareti |
| `mention` | `lightblue` | `@kişi`, `#kanal` |
| `mention_role` | `lightmagenta` | `@rol`, `@everyone`, `@here` |
| `mention_me_bg` | `#3c3200` | Seni anan mesajda anmanın arka planı |
| `link` | `cyan` | Bağlantılar (altı çizili) |
| `highlight` | `lightblue` | Bağlantılı embed başlığı, kendi tepkilerin |
| `code` | `yellow` | Satır içi kod, kod bloğu |
| `emoji` | `yellow` | `:özel_emoji:` adları |
| `spoiler` | `darkgray` | Gizli (spoiler) metin |
| `selection_bg` | `#2d2d3c` | Seçili mesajın arka planı |
| `popup_bg` | `#1e1e28` | Tamamlama penceresi adayları |
| `channel` | `gray` | Okunmuş kanal adları |
| `unread` | `reset` (kalın) | Okunmamış kanal/sunucu |
| `current` | `green` | Açık kanal ve sunucu |
| `voice_speaking` | `green` | Konuşan kişi (kalın) |
| `badge` | `red` | Mention sayacı (kalın) |
| `error` | `red` | Hata mesajları, gönderilemeyen mesaj |
| `warning` | `yellow` | Bildirimler, yeniden bağlanma, silme onayı |
| `success` | `green` | Bağlı durumu, ses bağlantısı |
| `status_fg` / `status_bg` | `reset` | Durum çubuğu |

`timestamp`, `dim` ve `quote` yuvalarında `reset` "renk yok, soluk biçim" demektir; bir renk
verilirse soluk biçim yerine o renk kullanılır. Seçili satır/mesaj vurgusu (ters renk) ve kalın/italik
gibi biçimler temadan bağımsızdır. Embed'in kendi rengi (sol kenar) Discord'dan gelir.

### Ayarlar ekranı ve komutlar

- `:settings`: tam ekran; `ui.*`, `behavior.*`, `notifications.*`, `voice.*` ayarlarını mevcut
  değerleriyle listeler. `j`/`k` gez, `Enter`/`Boşluk` açık-kapalı ya da sonraki seçenek, `+`/`-`
  (`←`/`→`) sayı ve seçenek, `s` kaydet, `Esc` kapat.
- Satır sonundaki etiket: **anında geçerli** (`behavior.send_typing`, `behavior.send_read_receipts`,
  `notifications.enabled`, `notifications.on`: denetleyiciye `Action` olarak gider) ya da
  **yeniden başlatınca geçerli** (diğerleri). `voice.vad_threshold` da anında geçerlidir (açık
  ses oturumuna atomik olarak yazılır). `ui.theme` salt okunurdur; `:theme <ad>` kullanılır.
  `voice.input_device`/`output_device` salt okunurdur (listeyi almak ses sürücüsünü yoklar, UI
  thread'inde yapılmaz): adlar `:devices` ile görülüp config.toml'a yazılır.
- `s`: sadece bu oturumda değişenler `config.toml`'a yazılır (`toml_edit`: yorumlar, sıra ve biçim
  korunur; olmayan bölüm/anahtar eklenir). Disk işi denetleyicide, geçici dosya + yer değiştirme ile;
  sonuç durum çubuğunda bildirilir. `client.*` ve `[keys]` ekranda yok (dosyadan düzenlenir).
- `:theme <ad>`: tema dosyasını denetleyici okur, sonuç TUI'ye iletilir ve anında uygulanır;
  `ui.theme` kaydedilecek olarak işaretlenir (`:settings` → `s`).
