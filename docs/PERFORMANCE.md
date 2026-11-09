# Performans profili (sentetik büyük hesap)

Bu ölçüm **sentetik veriyle** yapıldı; Discord'a bağlanılmadı, token kullanılmadı. Gerçek bir
hesapta canlı ölçüm **henüz yapılmadı** (bekliyor).

## Veri

`crates/digcord-tui/src/perf_synth.rs` (yalnızca test derlemesi, deterministik, dosya commit'lenmez):
100 sunucu, 4500 kanal (+500 kategori; 5000 kanal nesnesi), 400 rol, 2000 kullanıcı, 300 DM,
4800 read_state girdisi, sunucu bildirim ayarları (kanal geçersiz kılmalarıyla), 400 ilişki,
sunucu başına 20 emoji ve ses durumları. READY JSON ≈ 1,9 MiB.
Mesajlar: 50 kanal x 200 mesaj (≈ 4,7 MiB JSON); ~%12 ek, ~%17 gömme, ~%10 tepki, markdown,
bağlantı, bahsetme.

## Çalıştırma

```
cargo test -p digcord-tui --release -- --ignored --nocapture large_account_profile
```

Bellek `/proc/self/statm` RSS'inden okunur (`unsafe` yok, sayaçlı ayırıcı yok). Üretim çöpü
ölçümü bozmasın diye veri ayrı iş parçacığında üretilir; taban RSS üretimden sonra alınır
(üretilmiş JSON baytlarını da içerir). RSS sayacı serbest bırakılan belleği hemen işletim
sistemine geri vermeyen `malloc` yüzünden kaba bir üst sınırdır.

## Sonuçlar

Makine: Intel i3-12100F (8 mantıksal çekirdek), 15 GiB RAM, CachyOS, Rust `--release`.
Birkaç çalıştırmanın tipik değerleri:

| Ölçüm | Sonuç |
|---|---|
| (a) READY JSON -> `Ready` ayrıştırma | ~5-6 ms |
| (b) `State::load_ready` | ~2-3 ms |
| (c) READY sonrası bellek (ayrıştırma + state, Ready bırakıldıktan sonra) | +4 MiB |
| 50x200 mesaj JSON ayrıştırma (10k mesaj) | ~15 ms |
| (d) 10k mesaj sonrası bellek | +8 MiB (toplam RSS ≈ 50 MiB, test sürecinde) |
| (e) bir TUI karesi (200x60, TestBackend), soğuk / sıcak | ~1,4 ms / ~0,75 ms |

Sonuç: açılışta veri işleme (ayrıştırma + yükleme) toplam < 10 ms; bellek hedefi (< 150 MB)
sentetik büyük hesapta rahat sağlanıyor. Bariz israf bulunmadı, bu yüzden **optimizasyon
yapılmadı**.

## Kapsam dışı / bekleyen

- Gerçek hesapla canlı ölçüm (gerçek payload'lar, lazy guild üye listeleri, op 37 abonelikleri).
- Medya/resim önbelleği, ses (DAVE/opus) ve tokio çalışma zamanı belleği bu testte yok.
- Gerçek ikili için açılış süresi (süreç başlatma, keyring, TLS el sıkışması, ağ) ölçülmedi.
