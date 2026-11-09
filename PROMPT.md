# Başlangıç promptu

Aşağıdaki metni proje klasöründe kodlama ajanına (Claude Code vb.) ilk mesaj olarak ver.

---

Digcord adında, tamamen Rust ile yazılmış bir Discord TUI istemcisi geliştiriyoruz.
Bu klasördeki `CLAUDE.md`, `README.md` ve `docs/` altındaki tüm dosyaları baştan sona oku.

Özet:
- Kullanıcı token'ı ile çalışan, terminalde çalışan tam bir Discord istemcisi.
- Gateway, REST ve ses katmanı sıfırdan yazılacak; Discord'a özel kütüphane yok.
- ratatui + crossterm arayüz, tokio, rustls.
- Özellikler: sunucular/kanallar/mesajlaşma, DM ve grup DM, okunmamış + bildirimler,
  terminal içi resim gösterimi (Kitty/Sixel/yarım blok), DAVE uçtan uca şifrelemeli sesli kanal.
- Hedef: Linux (CachyOS, KDE Plasma, PipeWire). Performans öncelikli.
- Video, ekran paylaşımı ve etkinlikler ana kapsamda değil; sadece mimari ileride eklenebilir kalsın.

- Lisans tescilli (`LICENSE`, tüm hakları saklıdır); GPL/AGPL bağımlılık ekleme.
- Proje adım adım ilerleyecek ve her adım GitHub'a commit + push edilecek (`docs/GIT_WORKFLOW.md`).

Şimdi **sadece Aşama 0**'ı yapacağız (`docs/ROADMAP.md`):
1. `git` ve `gh` kurulu ve giriş yapılmış mı kontrol et; değilse bana ne yapmam gerektiğini söyle.
2. `git init -b main`, mevcut dokümanları ilk commit yap ve `gh repo create digcord --private --source=. --remote=origin --push`
   ile **private** depo oluşturup push et. Depo private değilse dur ve bana söyle.
3. `asama-0-iskelet` branch'ini aç.
4. Bana Aşama 0 için kısa bir plan ve kullanacağın bağımlılıkların listesini (gerekçeleri ve lisanslarıyla) ver.
5. Onay verince ilerle; her çalışan adımda commit + push yap:
   - workspace ve crate iskeletleri (`license-file`, `publish = false`)
   - log altyapısı (dosyaya, token maskeleme ile) ve panic hook
   - boş üç panelli ratatui ekranı
   - fmt + clippy + test için GitHub Actions iş akışı
6. Aşama 0 kabul kriterlerini tek tek kontrol et, `ROADMAP.md`'de işaretle, `main`'e birleştir,
   `v0.1.0` etiketini at ve push et. Sonucu raporla.

Aşama 0 bitmeden gateway veya ağ koduna başlama. Belirsiz bir şey olursa tahmin etme, sor.
