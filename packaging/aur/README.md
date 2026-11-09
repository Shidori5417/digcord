# Digcord — AUR paketleme

Bu dizin `digcord-git` paketinin Arch Linux derleme dosyalarını içerir.

## Yerel derleme

```sh
cd packaging/aur/digcord-git
makepkg -si
```

- `-s` eksik derleme bağımlılıklarını pacman ile kurar, `-i` ürettiği paketi kurar.
- Kaynak `https://github.com/Shidori5417/digcord.git` adresinden git HEAD'i olarak çekilir.
  Depo özel olduğu için git bu adrese kimlik doğrulamasıyla erişebilmelidir
  (ör. `gh auth setup-git` ile credential helper ayarı). Erişim yoksa `makepkg` kaynağı indiremez.
- Yeniden derleme: `makepkg -sf`. Sürüm `pkgver()` ile `rN.hash` biçiminde (ör. `r57.1a2b3c4`) git'ten hesaplanır.
- PKGBUILD değişirse `.SRCINFO` yeniden üretilmelidir:
  `makepkg --printsrcinfo > .SRCINFO`
- `makepkg` çalışma dizininde `src/`, `pkg/` ve `*.pkg.tar.zst` üretir. Bunlar git'e eklenmemelidir.

## Bağımlılıklar

- Derleme: `git`, `cargo` (Arch'ta `rust` paketi sağlar), `cmake` (libopus).
- Çalışma zamanı: `alsa-lib` (ses, cpal üzerinden).
- Opsiyonel:
  - `gnome-keyring`: token saklama için Secret Service sağlayıcısı (D-Bus).
  - `wl-clipboard` (Wayland) veya `xclip` (X11): pano desteği.
  - `xdg-utils`: bağlantıları tarayıcıda açmak için (`xdg-open`).
  - Bir bildirim daemon'u (ör. `dunst`): masaüstü bildirimleri için.

## AUR'a yayınlama neden yapılmıyor

Proje tescillidir (`LICENSE`: tüm hakları saklıdır) ve GitHub deposu özeldir.
AUR'a yüklemek paketi herkese açık olarak dağıtmak anlamına gelir; bu lisansla ve depo
görünürlüğüyle uyumlu değildir. Bu nedenle PKGBUILD AUR'a gönderilmez, hiçbir yere
yayınlanmaz; dizin yalnızca yerel derleme içindir.
