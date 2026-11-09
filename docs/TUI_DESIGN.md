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
| `Ctrl+Shift+M` / `Ctrl+Shift+D` | Mikrofon / kulaklık sessize al |
| `Alt+↑/↓` | Okunmamış önceki/sonraki kanal (sessizdekiler atlanır) |
| `q` / `:q` | Çıkış |

Tüm kısayollar `config.toml` içinden değiştirilebilir.

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

- Kanal ağacında ses kanalındaki kullanıcılar; konuşan kişi vurgulanır (speaking olayları).
- Alt çubuk: bağlı kanal, mikrofon/kulaklık durumu, gecikme, şifreleme durumu (DAVE ✓).
- Kullanıcıya tıklama/seçme → ses seviyesi kaydırıcısı, yerel susturma.
- Ses ayarları penceresi: giriş/çıkış cihazı, PTT tuşu, VAD eşiği.

## Config

`~/.config/digcord/config.toml`

**Şu an desteklenenler:** `ui.show_member_list`, `ui.images`, `ui.image_max_rows`, `behavior.send_typing`,
`behavior.send_read_receipts`, `notifications.enabled`, `notifications.on` (`"mention"`, `"dm"`,
`"all"`), `client.build_number`, `client.chrome_major`. Diğer anahtarlar okunur ama yok sayılır
(açılışta uyarı); bozuk dosyada uygulama açılmaz ve hatayı gösterir.

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
input_device = "default"
output_device = "default"
mode = "vad"             # "vad" | "ptt"
ptt_key = "ctrl+space"

[client]
# İstemci kimliği profili — bkz. docs/PROTOCOL_NOTES.md
build_number = 0          # 0 = otomatik/varsayılan

[keys]
# "eylem" = "tuş" geçersiz kılmaları
```

Token config'de **tutulmaz** (keyring).
