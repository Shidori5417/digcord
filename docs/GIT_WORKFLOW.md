# Git ve GitHub İş Akışı

Proje adım adım ilerler; her anlamlı adım commit'lenir ve GitHub'a push'lanır.
Depo **gizli (private)** tutulur.

## İlk kurulum (bir kez)

Ön koşul: `git` ve GitHub CLI (`gh`) kurulu, `gh auth login` yapılmış.
CachyOS'ta: `sudo pacman -S git github-cli`

```bash
cd digcord
git init -b main
git add .
git commit -m "docs: başlangıç dokümanları, lisans ve git iş akışı"
gh repo create digcord --private --source=. --remote=origin --push
```

Kontrol: `gh repo view --web` → depo sayfasında "Private" etiketi görünmeli.

## Branch düzeni

- `main` — her zaman derlenen, testleri geçen kod. Doğrudan büyük iş yapılmaz.
- Her aşama veya özellik için ayrı branch:
  - `asama-0-iskelet`, `asama-1-gateway`, `asama-2-okuma` ...
  - Küçük düzeltmeler: `fix/heartbeat-zombi`, `docs/protokol-notu`
- Aşama bitince (kabul kriterleri sağlanınca) `main`'e birleştirilir:
  ```bash
  gh pr create --base main --fill   # gizli depoda da PR kullanılabilir, kayıt tutar
  gh pr merge --squash --delete-branch
  ```
  PR istemiyorsan: `git switch main && git merge --no-ff asama-0-iskelet && git push`

## Ne zaman commit?

- Her **çalışan küçük adım** bir commit: derleniyor, `cargo fmt` + `cargo clippy -D warnings` temiz,
  ilgili testler geçiyor.
- Yarım/derlenmeyen kod `main`'e girmez; özellik branch'inde ara commit serbest.
- Bir commit tek bir iş yapar (ör. "heartbeat eklendi" ile "README düzeltmesi" ayrı).

## Ne zaman push?

- Her commit'ten sonra veya en geç çalışma oturumu sonunda push (yedek görevi de görür).
- Push öncesi: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

## Commit mesajı biçimi

Conventional Commits, açıklama Türkçe:

```
<tür>(<kapsam>): <kısa açıklama>

<isteğe bağlı ayrıntı>
```

| Tür | Kullanım |
|---|---|
| `feat` | yeni özellik |
| `fix` | hata düzeltme |
| `perf` | performans |
| `refactor` | davranış değiştirmeyen yeniden düzenleme |
| `test` | test ekleme/düzeltme |
| `docs` | doküman |
| `chore` | bağımlılık, CI, araç |

Kapsamlar crate adlarıyla: `gateway`, `http`, `store`, `media`, `voice`, `notify`, `tui`, `model`.

Örnekler:
```
feat(gateway): zlib-stream açıcı ve Hello/Identify akışı
fix(http): 429 sonrası global kilit serbest bırakılmıyordu
perf(tui): mesaj satır yüksekliklerini önbelleğe al
```

## Etiketler (sürümler)

Her aşama `main`'e girince etiket:
```bash
git tag -a v0.1.0 -m "Aşama 0: iskelet"
git push --tags
```
Aşama N → `v0.(N+1).0`. Ses aşaması sonrası ilk kararlı hedef `v1.0.0`.

## Asla commit'lenmeyecekler

- Token, `.env`, keyring dökümleri
- Anonimleştirilmemiş gateway/REST kayıtları (`/captures/` — `.gitignore`'da)
- Gerçek kullanıcı adı, ID veya mesaj içeren test dosyaları

Yanlışlıkla token commit'lenirse: **önce token'ı geçersiz kıl** (Discord'da şifre değiştir →
tüm oturumlar düşer), sonra geçmişten temizle (`git filter-repo`). Gizli depo olması bunu
gereksiz kılmaz.

## Gizlilik kontrolü

- Depo private kalmalı; görünürlüğü değiştirmeden önce `LICENSE`'ı tekrar düşün.
- Collaborator eklenirse bile lisans gereği kodu kullanma/geliştirme hakkı olmaz;
  yine de sadece güvendiğin kişileri ekle.
- `Cargo.toml` içinde `publish = false` → yanlışlıkla crates.io'ya yüklenmez.
