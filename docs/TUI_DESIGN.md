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
 [NORMAL]  Digcord · bağlı · 12 sunucu          Ayşe yazıyor...      42 ms · 14:03
```

- **Sunucu şeridi** (en sol, dar): DM + sunucu kısaltmaları/ikonları, okunmamış noktası, mention sayısı.
- **Kanal ağacı:** kategoriler katlanabilir, okunmamış kalın, mention sayısı sağda. Altta ses durumu çubuğu.
- **Mesaj görünümü:** gruplanmış mesajlar (aynı yazar, < 7 dk), yanıt önizlemesi, tepkiler, ekler, embed'ler;
  gün ayırıcıları, "Yeni mesajlar" çizgisi, rol renkli yazar adları, `BOT` rozeti, renkli kod blokları
  (ayrıntı: "Mesaj işleme").
- **Üye listesi** (sağ panel, 20 sütun): sunucu kanalında gateway'in üye listesi (op 37 `channels`
  aboneliği, `GUILD_MEMBER_LIST_UPDATE`); `ROLADI — N` / `Çevrimiçi — N` / `Çevrimdışı — N` başlıkları
  altında üyeler, ad önceliği takma ad > global ad > kullanıcı adı, rengi en yüksek renkli rolden,
  solda durum işareti (`●` çevrimiçi yeşil, boşta sarı, rahatsız etmeyin kırmızı; `○` çevrimdışı soluk).
  Uzun/geniş karakterli adlar `…` ile kısaltılır. DM/grup DM'de kanalın alıcıları (işaretsiz). İsteğe bağlı:
  `Alt+M` (`toggle_members`) ya da `:members` aç/kapat; açılış durumu `ui.show_member_list`; dar
  terminalde otomatik gizlenir. İlk sürümde yalnızca ilk 100 öğe (kaydırma yok). Renkler tema
  yuvaları `member_group`, `presence_online|idle|dnd|offline`.
- **Mesaj paneli başlığı** (panelin ilk iç satırı; panel çok kısaysa çizilmez): solda kanal simgesi ve
  adı (`# sohbet`, thread `└`, ses `🔊`, forum `≡`, DM/grup DM `@ ad, ad`) ve soluk renkte konunun ilk satırı
  (sığmazsa `…` ile kısalır); sağda, öncelik sırasıyla: `[geçmiş]` (yukarı kaydırılmışken), `🔊` (açık
  ses oturumu bu sunucuda; DM'de bu kanalda), `● çevrimiçi/toplam` (gateway üye listesinden; liste henüz
  gelmediyse yok), `📌 N` (yalnızca bu oturumda `:pins` çalıştırıldıysa bilinen sabit sayısı; ayrıca
  getirilmez). Dar genişlikte önce en düşük öncelikli parça düşer, sonra konu, en son kanal adı kısalır.
  DM'de alıcıların çevrimiçi durumu store'da tutulmadığı için gösterilmez.
- **Bildirim kutuları (toast):** mesaj alanının sağ üstünde, başlık satırının altında üst üste, en yenisi
  üstte, en çok 3; her biri ~4 sn sonra kendiliğinden kaybolur (döngü `recv_timeout` ile tam o anda
  uyanır; boşta CPU harcamaz). Kaynaklar: açık olmayan kanaldaki yeni DM / anma (masaüstü bildirimiyle
  aynı sessize alma ve `notifications.*` ayarları; kanal açıksa kutu çıkmaz) ve denetleyiciden gelen
  hatalar/uyarılar (`gönderilemedi`, `yüklenemedi`, ayar kaydı vb.; durum çubuğunda da kalır). Kapatmak:
  `ui.toasts = false` (`:settings`'ten anında). Olay: `UiEvent::Toast { title, body, kind }`. Renkler tema
  yuvaları `toast_bg`, `toast_info` (hata/uyarı `error`/`warning`).
- **Durum çubuğu:** solda mod, bağlantı durumu, ses oturumu, bildirim/yazıyor bilgisi; sağda gateway
  gecikmesi (`42 ms`, heartbeat → ACK; < 150 yeşil, < 400 sarı, üstü kırmızı; yalnızca bağlıyken) ve
  yerel saat `SS:DD` (dakika değişince kendiliğinden yenilenir). Dar genişlikte önce gecikme, sonra
  saat düşer; sonra bildirim/ses metni `…` ile kısalır; mod ve bağlantı durumu en son.

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
| `Alt+M` / `:members` | Üye listesi aç/kapat (`Ctrl+M` terminalde Enter ile aynı olduğundan kullanılamaz; `toggle_members` ile atanabilir) |
| `Ctrl+Shift+M` / `Ctrl+Shift+D` | Mikrofon / kulaklık sessize al (kitty klavye protokolü gerekir; yoksa `:mute` / `:deaf`) |
| `Enter` (ses kanalında) | Ses kanalına katıl |
| `Ctrl+Space` (basılı tut) | Bas-konuş (`voice.mode = "ptt"`; kitty klavye protokolü gerekir) |
| `:leave` / `:mute` / `:deaf` | Ses kanalından ayrıl / mikrofon / kulaklık |
| `:devices` | Kullanılabilir ses giriş/çıkış aygıtlarını listele (durum çubuğuna tek satır; tam liste günlükte) |
| `:call` | Açık DM'de aramaya katıl (çaldırmaz) |
| `:threads` | Seçili/açık kanalın thread listesini aç-kapat (forumda gönderileri de getirir) |
| `:archived` | Seçili/açık kanalın arşivli thread'lerini yükle ve soluk göster |
| `:search <sorgu>` | Sunucuda (DM'de o DM'de) mesaj ara; tam ekran sonuç listesi. Süzgeçler: `from:<kimlik\|<@id>\|@ad>`, `in:#kanal`, `has:link\|file\|image\|embed`; `"tırnaklı"` sözcük süzgeç sayılmaz |
| `:pins` | Açık kanalın sabitlenmiş mesajları (aynı sonuç ekranı) |
| `:vol <ad> <0-200>` | Ses kanalındaki kişinin ses seviyesi (yüzde) |
| `Alt+↑/↓` | Okunmamış önceki/sonraki kanal (sessizdekiler atlanır) |
| `?` / `:help` | Yardım: tüm kısayollar ve komutlar (aşağıda) |
| `q` / `:q` | Çıkış |

Normal/SELECT modu ve genel kısayollar `config.toml` `[keys]` tablosundan değiştirilebilir (aşağıda).
`Ctrl+C` (çıkış), INSERT/COMMAND metin düzenleme tuşları, hızlı geçiş/resim görüntüleyici/silme onayı
pencerelerinin içindeki tuşlar ve ayarlar ekranının tuşları sabittir. Mesaj panelinin altındaki ipucu
satırı (`i: yaz · v: seç · ?: yardım · :upload <yol>` ve SEÇ modundaki karşılığı) canlı kısayol
tablosundan üretilir: `[keys]` ile değişen tuşlar orada da görünür.

### Yardım ekranı (`?`, `:help`)

Tam ekran, `[YARDIM]` modu. Kısayollar **canlı `Keymap`'ten** üretilir (yani `[keys]` geçersiz
kılmaları doğru görünür; atanmamış eylem `(atanmamış)` yazar): *Genel*, *NORMAL*, *SEÇ* tabloları
(tuşlar, açıklama, köşeli parantezde `[keys]` eylem adı), ardından yeniden atanamayan tuşlar
(*Sabit*, *YAZ*, *Resim görüntüleyici*, *Arama / sabitlenenler*, *Ayarlar*, *Yardım*) ve en altta
komutlar (`:upload`, `:search`, `:pins`, `:threads`, `:archived`, `:members`, `:settings`, `:theme`,
`:devices`, `:leave`, `:mute`, `:deaf`, `:call`, `:vol`, `:help`, `:q`). `j`/`k` (`↓`/`↑`) kaydır,
`PgUp`/`PgDn` sayfa, `Home`/`End` başa/sona, `Esc`/`q` kapat; SEÇ modundan açılmışsa aynı mod ve seçim
korunarak dönülür. YAZ modunda `?` düz karakterdir.

## Mesaj işleme

- Markdown alt kümesi: **kalın**, *italik*, __altı çizili__, ~~üstü çizili~~, `kod`, kod blokları
  (dil adına göre sözdizimi renklendirme, aşağıda), `> alıntı`, `||spoiler||` (gizli, `o` ile aç),
  başlıklar, listeler.
- Mention'lar (`<@id>`, `<#id>`, `<@&rol>`) isimle ve renkle; kendine mention vurgulu arka plan.
- Özel emoji: satır içinde `:isim:` (terminal hücre yapısı yüzünden satır içi resim yok);
  sadece emojiden oluşan mesajlarda (en fazla 27) emojiler büyük resim olarak yan yana.
- Unicode emoji genişliği `unicode-width` ile; hizalama bozulmalarına dikkat.
- Zaman damgaları `<t:...:R>` biçimleri yerelleştirilmiş gösterilir.
- **Gün ayırıcısı:** yerel saat dilimine göre günü değişen mesajın (ve yüklü ilk mesajın) üstünde ortalı,
  soluk `── 11 Kasım 2026 ──` (Türkçe ay adları). Ayırıcıdan sonra mesaj başlığı yeniden yazılır
  (gruplama kesilir). Satırlar mesajın kendi yüksekliğine dahildir: sanal kaydırma (çapa + ofset)
  aynı `view::lines` yolundan geçtiği için yükseklikler çizimle birebir tutarlıdır.
- **"Yeni mesajlar" çizgisi:** kanal açılırken okunmamış mesaj varsa, *açılış anındaki* okuma işaretinden
  yeni ilk mesajın üstünde ortalı `── Yeni mesajlar ──` (`new_messages` yuvası, varsayılan kırmızı).
  Canlı okuma işareti ilerleyince silinmez; başka kanal açılınca ya da `G`/`End` ile silinir. Yüklü
  pencerenin en eski mesajı ilk okunmamışsa (öncesi yüklü değil) çizilmez.
- **Yazar:** sunucu kanalında adın rengi üyenin rengi olan en yüksek rolüdür (`State::author_color`;
  üye rolleri mesajın `member` alanından, üye listesinden ve chunk'lardan gelir); DM'de ve rol
  bilinmiyorsa `author` yuvası. `user.bot` olan yazarın adından sonra `BOT` rozeti (`bot_badge`).
  Sistem mesajları değişmeden `→ metin  saat` biçimindedir.
- **Yanıt:** yanıtın üstünde soluk `╭─ Ad: metin` (ilk satır, genişliğe göre `…` ile kısalır; anmalar
  normal içerik gibi çözülür ve renklerini korur). Hedef silinmişse `╭─ (silinmiş mesaj)`.
- **Kod bloğu renklendirme:** çitin dili (`rust`, `py`, `js`, `ts`→js, `json`, `bash`/`sh`…) `syntect`
  (MIT; saf Rust `regex-fancy`, C bağımlılığı yok) ile renklendirilir; renkler `Color::Rgb`. Bilinmeyen
  dil, dilsiz çit, 100 satır / 8 KiB'den büyük blok ve satır içi kod eski düz `code` stilinde kalır.
  Sözdizimi/tema dökümleri açılışta arka plan iş parçacığında yüklenip ısıtılır (yüklenene kadar düz
  çizilir); sonuçlar kod özetine göre önbelleğe alınır (isabet ≈ 0,5 µs). Renklendirme sarmayı
  değiştirmez, yani satır sayısı/yükseklik aynıdır. İlk renklendirme (önbellek ıskası) 40 satırda
  ≈ 9 ms (release). Ayarlar: `ui.code_highlight` (`true`/`false`), `ui.code_theme` (`base16-ocean.dark`
  varsayılan, `base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.light`, `InspiredGitHub`,
  `Solarized (dark)`, `Solarized (light)`); ikisi de yeniden başlatınca geçerli.

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

- Mention, DM ve "tüm mesajlar" bildirimi açık kanallarda masaüstü bildirimi (notify-rust → KDE);
  ayrıca terminalde köşe bildirim kutusu (`ui.toasts`, yukarıda) — masaüstü bildiriminden farkı:
  açık kanal için hiç çıkmaz (odak durumuna bakılmaz).
- Sunucu/kanal sessize alma ayarlarına (READY'deki kullanıcı guild ayarları) uyulur.
- Terminal odaktaysa ve ilgili kanal açıksa bildirim gönderilmez.
- Opsiyonel terminal zili / pencere başlığında okunmamış sayısı.

## Ses arayüzü

- Kanal ağacında ses kanalındaki kullanıcılar; konuşan kişi yeşil, susturma simgeleri
  (`🎤✕`, `🎧✕`). ✅
- Durum çubuğu: bağlı kanal, aşama (katılınıyor/bağlanıyor/bağlı), "uçtan uca" (DAVE),
  mikrofon/kulaklık durumu. ✅ Gecikme: yalnızca gateway (ses UDP gecikmesi yok).
- Kullanıcı ses seviyesi `:vol` ile. ✅ Kaydırıcı ve yerel susturma yok (sonra).
- Ses aygıtı ve VAD eşiği config'ten: `voice.input_device` / `voice.output_device` (cpal aygıt
  adı, tam eşleşme; boş ya da `default` = sistem varsayılanı; bulunamazsa uyarılıp varsayılana
  dönülür). Adlar `:devices` ile görülür. `voice.vad_threshold` (RMS, 0.001–0.5, varsayılan
  0.012) `:settings`'ten `+`/`-` ile anında değişir. Aygıt değişikliği bir sonraki katılımda
  geçerli. Ayrı bir ses ayarları penceresi ve ayarlanabilir PTT tuşu yok: Ctrl+Space sabit. (sonra)

## Config

`~/.config/digcord/config.toml`

**Şu an desteklenenler:** `ui.theme`, `ui.show_member_list`, `ui.toasts`, `ui.images`, `ui.image_max_rows`,
`ui.code_highlight`, `ui.code_theme`,
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
toasts = true            # köşe bildirimleri (yeni DM/anma, hatalar)
images = "thumbnail"     # "off" | "thumbnail" | "full"
image_max_rows = 10
code_highlight = true     # kod bloklarını dile göre renklendir
code_theme = "base16-ocean.dark"

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
| genel | `toggle_members` | `alt+m` | Üye listesini aç/kapat |
| NORMAL | `quit` | `q` | Çıkış |
| NORMAL | `switcher` | `ctrl+k` | Hızlı geçiş |
| NORMAL | `next_focus` / `prev_focus` | `tab` / `shift+tab` | Panel odağı |
| NORMAL | `activate` | `enter`, `l` | Sunucu/kanal aç; mesaj panelinde `enter` SELECT'e girer |
| NORMAL | `insert_mode` | `i` | Yazma modu |
| NORMAL | `select_mode` | `v` | Mesaj seçme modu |
| NORMAL | `command_mode` | `:` | Komut modu |
| NORMAL | `page_down` / `page_up` | `pagedown`, `ctrl+d` / `pageup`, `ctrl+u` | Yarım sayfa kaydır |
| NORMAL | `scroll_bottom` | `G`, `end` | En alta in (aramadan eski mesaja atlanmışsa güncel mesajları yeniden yükler) |
| NORMAL | `next_unread` / `prev_unread` | `alt+down` / `alt+up` | Okunmamış kanala atla |
| NORMAL+SELECT | `move_down` / `move_up` | `j`, `down` / `k`, `up` | Gezin / mesaj seçimini kaydır |
| SELECT | `back` | `esc` | NORMAL'e dön |
| SELECT | `reply` / `edit` | `r` / `e` | Yanıtla / düzenle |
| SELECT | `delete` | `d` | İki kez basılınca silme onayı (`d d`) |
| SELECT | `react` | `+` | Tepki istemi |
| SELECT | `copy` | `y` | Metni kopyala |
| SELECT | `open_media` | `o` | Resmi/bağlantıyı aç |
| SELECT | `retry` | `enter` | Başarısız mesajı yeniden gönder |
| NORMAL+SELECT | `help` | `?` | Yardım ekranı (`:help` ile de) |

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
| `author` | `cyan` | Mesaj yazarı (kalın); rol rengi varsa o kullanılır |
| `timestamp` | `reset` (soluk) | Saatler, `<t:…>` zaman damgaları |
| `dim` | `reset` (soluk) | İkincil metin, ipuçları, yanıt önizlemesi, ekler |
| `quote` | `reset` (soluk) | Alıntı (`>`) işareti |
| `mention` | `lightblue` | `@kişi`, `#kanal` |
| `mention_role` | `lightmagenta` | `@rol`, `@everyone`, `@here` |
| `mention_me_bg` | `#3c3200` | Seni anan mesajda anmanın arka planı |
| `link` | `cyan` | Bağlantılar (altı çizili) |
| `highlight` | `lightblue` | Bağlantılı embed başlığı, kendi tepkilerin |
| `code` | `yellow` | Satır içi kod, renklendirilmeyen kod bloğu |
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
| `new_messages` | `red` | "Yeni mesajlar" çizgisi (kalın) |
| `bot_badge` | `blue` | Bot yazarların `BOT` rozeti (kalın) |
| `toast_bg` | `#1e1e28` | Bildirim kutusu arka planı |
| `toast_info` | `cyan` | Mesaj/bilgi bildirimi çerçevesi ve başlığı (hata: `error`, uyarı: `warning`) |
| `status_fg` / `status_bg` | `reset` | Durum çubuğu |
| `member_group` | `reset` | Üye listesi grup başlıkları (soluk) |
| `presence_online` / `presence_idle` / `presence_dnd` / `presence_offline` | `green` / `yellow` / `red` / `darkgray` | Üye listesi durum işareti |

`timestamp`, `dim` ve `quote` yuvalarında `reset` "renk yok, soluk biçim" demektir; bir renk
verilirse soluk biçim yerine o renk kullanılır. Seçili satır/mesaj vurgusu (ters renk) ve kalın/italik
gibi biçimler temadan bağımsızdır. Embed'in kendi rengi (sol kenar) Discord'dan gelir.

### Ayarlar ekranı ve komutlar

- `:settings`: tam ekran; `ui.*`, `behavior.*`, `notifications.*`, `voice.*` ayarlarını mevcut
  değerleriyle listeler. `j`/`k` gez, `Enter`/`Boşluk` açık-kapalı ya da sonraki seçenek, `+`/`-`
  (`←`/`→`) sayı ve seçenek, `s` kaydet, `Esc` kapat.
- Satır sonundaki etiket: **anında geçerli** (`behavior.send_typing`, `behavior.send_read_receipts`,
  `notifications.enabled`, `notifications.on`, `ui.toasts`: denetleyiciye `Action` olarak gider) ya da
  **yeniden başlatınca geçerli** (diğerleri). `voice.vad_threshold` da anında geçerlidir (açık
  ses oturumuna atomik olarak yazılır). `ui.theme` salt okunurdur; `:theme <ad>` kullanılır.
  `voice.input_device`/`output_device` salt okunurdur (listeyi almak ses sürücüsünü yoklar, UI
  thread'inde yapılmaz): adlar `:devices` ile görülüp config.toml'a yazılır.
- `s`: sadece bu oturumda değişenler `config.toml`'a yazılır (`toml_edit`: yorumlar, sıra ve biçim
  korunur; olmayan bölüm/anahtar eklenir). Disk işi denetleyicide, geçici dosya + yer değiştirme ile;
  sonuç durum çubuğunda bildirilir. `client.*` ve `[keys]` ekranda yok (dosyadan düzenlenir).
- `:theme <ad>`: tema dosyasını denetleyici okur, sonuç TUI'ye iletilir ve anında uygulanır;
  `ui.theme` kaydedilecek olarak işaretlenir (`:settings` → `s`).

### Thread'ler ve forum kanalları (Aşama 7)

- Aktif thread'ler kanal panelinde üst kanalın altında, bir seviye girintili ve `└ ` ile gösterilir
  (yeniden eskiye, son mesaja göre). Thread'ler guild ağacında (`TreeItem`) yoktur; panel
  `State::threads_of` ile üst kanalın altına ekler. Üst kanal gizliyse (izin) thread'leri de görünmez.
- **Varsayılan açık/kapalı:** metin/duyuru kanalında en çok 3 aktif thread varsa açık, daha fazlaysa
  kapalı (panel şişmesin); forumda her zaman kapalı. Kapalıyken üst satırda `▸N` ipucu ve
  thread'lerin okunmamış/bahsetme toplamı görünür. `:threads` ters çevirir (forumda Enter de);
  bir thread açılınca (Alt+↓, hızlı geçiş vb.) listesi kapalıysa açılır.
- **Forum / medya** (`≡`): Enter mesaj açmaz, gönderi listesini açar; ilk açılışta
  `Action::LoadThreads` ile aktif gönderiler REST'ten alınır (`threads/search`, bkz.
  PROTOCOL_NOTES §4.4). Gönderi (thread) normal kanal gibi açılır: geçmiş, gönderme, düzenleme,
  tepki, yükleme aynı yollar.
- **Arşivli** thread'ler varsayılan gizli; `:archived` ilk sayfayı (25) yükler, soluk çizilir.
  **Kilitli** thread'ler `🔒` ve soluk; yazılamaz: `i` bildirim verir, giriş alanı
  "(bu thread kilitli; yazamazsın)" gösterir (`MANAGE_THREADS` sahibi yazabilir).
- İzinler üst kanaldan miras alınır; yazmak `SEND_MESSAGES_IN_THREADS` ister.
- Okunmamış/bahsetme sayaçları thread id'siyle mevcut okunma durumu mantığından gelir; sunucu
  rozeti ve `Alt+↑/↓` thread'leri de kapsar (sessizdekiler ve arşivliler hariç).
- **Kapsam dışı:** thread/forum gönderisi oluşturma, thread'e katılma/ayrılma, etiket süzme,
  arşivli listede sayfalama.

## Arama ve sabitlenenler (Aşama 7)

- `:search <sorgu>` ve `:pins` mod `[ARAMA]` ile tam ekran sonuç listesi açar (ayarlar/hızlı
  geçiş gibi). Satır: `#kanal  yazar  gg.aa.yyyy ss:dd  tek satır içerik` (DM'de `@kişi`;
  boş içerik "(ek veya embed)"). Sabit tuşlar: `j/k` (`↓/↑`) gez, `g/G` ilk/son, `Enter` mesaja
  git, `n/p` (`PgDn/PgUp`) sonraki/önceki 25'lik sayfa (yalnızca aramada), `Esc`/`q` kapat.
- Her komut ve her `n`/`p` tek istek (`Action::Search` / `Action::LoadPins`); yanıt
  `UiEvent::SearchResults`/`SearchFailed` ile gelir. İstekler `seq` taşır, eskimiş yanıt yok
  sayılır; yükleme sürerken `n`/`p` yok sayılır. 202 (dizin hazır değil) otomatik denenmez,
  ekranda "Arama dizini hazırlanıyor, birazdan tekrar dene" görünür. Sonuç verisi TUI'dedir,
  store'a yazılmaz (tek yazar kuralı: yazan yalnızca atlama için `around` penceresidir).
- `from:` kimlik, `<@id>` ya da store'da bilinen kullanıcı adı/görünen ad; `in:` kanal adı
  (mevcut sunucuda, büyük/küçük harf duyarsız) ya da `<#id>`/kimlik. Çözülemeyen süzgeç
  durum çubuğunda hata verir, istek atılmaz. DM'de `in:` kullanılamaz.
- **Atlama (`Enter`)**: mesaj o kanalda yüklüyse kanal açılır (gerekirse) ve mesaj SEÇ modunda,
  ekranın ortasına yakın seçilir; yüklü değilse `Action::JumpTo` ile `around=` penceresi
  getirilir (`PROTOCOL_NOTES.md` §4.5) ve store değişince aynı hizalama yapılır. Eski bir
  pencerede (ayrık) yeni mesajlar eklenmez, durum çubuğunda "güncele dönmek için G" notu çıkar,
  `i` ile yazma kapalıdır; `G` ilk sayfayı yeniden yükler. Kanaldan çıkınca ayrık pencere atılır.
- Hedef mesaj getirilemezse/silinmişse `UiEvent::JumpFailed` bekleyen atlamayı iptal eder.
