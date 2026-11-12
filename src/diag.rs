//! Yerel teşhis günlüğü: `state_dir/digcord/diag.log` (0600, satır başına bir JSON nesnesi).
//!
//! Amaç, hesap/token ile ilgili bir sorun olduğunda (ör. 4004 kapanması) ağ davranışımızı
//! sonradan görebilmek: bağlantı açılışı, IDENTIFY/RESUME, kapanma kodları, REST hataları...
//!
//! Güvenlik: olaylar `tracing` ile `target: "diag"` altında, **alanları emit eden kodca tek tek
//! seçilerek** gönderilir; token, Authorization, çerez, mesaj içeriği ve kullanıcı adı alanı
//! yoktur. Bu katman ayrıca (savunma amaçlı) yasaklı alan adlarını atar, değerlerdeki token
//! benzeri dizeleri maskeler ve uzun değerleri keser. Diske yazım ayrı bir thread'de yapılır;
//! olay kuyruğu doluysa olay atılır (emit eden asla beklemez).
//!
//! 2xx REST yanıtları tek tek yazılmaz; rota şablonu başına sayılıp ara sıra `rest_counts`
//! satırı olarak yazılır.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Gateway/http/ses crate'lerinin kullandığı hedef.
pub const TARGET: &str = digcord_gateway::DIAG;

/// Dosya bu boyutu aşınca eski yarısı atılır.
const MAX_BYTES: u64 = 1_000_000;
/// Bir alan değerinin en fazla uzunluğu (karakter).
const VALUE_MAX: usize = 160;
const QUEUE: usize = 1024;
/// Sayaç satırı: en çok bu aralıkta bir (çıkışta da yazılır).
const COUNTS_EVERY: Duration = Duration::from_secs(600);

/// Hiçbir koşulda yazılmayan alan adları (emit eden kod zaten kullanmaz; ikinci güvence).
const DENIED_FIELDS: &[&str] = &[
    "token",
    "authorization",
    "auth",
    "cookie",
    "cookies",
    "content",
    "message",
    "username",
    "user_name",
    "name",
    "password",
    "secret",
    "session_id",
    "endpoint",
    "ip",
    "port",
    "url",
];

/// Diag dosyasının varsayılan yolu.
pub fn default_path() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("digcord").join("diag.log"))
}

enum Msg {
    Event {
        ts: String,
        fields: Map<String, Value>,
    },
    Stop,
}

/// `tracing` katmanı: `target == "diag"` olaylarını yazıcı thread'e iletir.
pub struct DiagLayer {
    tx: SyncSender<Msg>,
}

/// Düşürülünce bekleyen olayları yazıp thread'i bitirir.
pub struct DiagGuard {
    tx: SyncSender<Msg>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for DiagGuard {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Dosyayı açar (0600) ve yazıcı thread'i başlatır.
pub fn init(path: PathBuf) -> io::Result<(DiagLayer, DiagGuard)> {
    let log = LogFile::open(path, MAX_BYTES)?;
    let (tx, rx) = sync_channel(QUEUE);
    let handle = std::thread::Builder::new()
        .name("digcord-diag".into())
        .spawn(move || writer(log, &rx))?;
    Ok((
        DiagLayer { tx: tx.clone() },
        DiagGuard {
            tx,
            handle: Some(handle),
        },
    ))
}

impl<S: tracing::Subscriber> Layer<S> for DiagLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != TARGET {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        if !visitor.fields.contains_key("kind") {
            return;
        }
        let msg = Msg::Event {
            ts: Timestamp::now().to_string(),
            fields: visitor.fields,
        };
        // Dolu kuyrukta olay atılır; emit eden asla beklemez.
        let _: Result<(), TrySendError<Msg>> = self.tx.try_send(msg);
    }
}

#[derive(Default)]
struct FieldVisitor {
    fields: Map<String, Value>,
}

impl FieldVisitor {
    fn put(&mut self, field: &Field, value: Value) {
        let name = field.name();
        if DENIED_FIELDS.contains(&name) {
            return;
        }
        self.fields.insert(name.to_owned(), value);
    }

    fn text(&mut self, field: &Field, s: &str) {
        self.put(field, Value::String(sanitize(s)));
    }
}

impl Visit for FieldVisitor {
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, Value::from(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, Value::from(value));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, Value::from(value));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field, value);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let mut s = String::new();
        let _ = write!(s, "{value:?}");
        self.text(field, &s);
    }
}

/// Değeri maskeler (token benzeri dizeler) ve keser.
fn sanitize(s: &str) -> String {
    let masked = crate::logging::mask_tokens(s);
    match masked.char_indices().nth(VALUE_MAX) {
        Some((i, _)) => format!("{}…", &masked[..i]),
        None => masked.into_owned(),
    }
}

/// Boyut sınırlı, 0600 modlu ekleme dosyası.
struct LogFile {
    path: PathBuf,
    file: std::fs::File,
    size: u64,
    max: u64,
}

impl LogFile {
    fn open(path: PathBuf, max: u64) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = open_append(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            size,
            max,
        })
    }

    fn append(&mut self, line: &str) -> io::Result<()> {
        if self.size + line.len() as u64 + 1 > self.max {
            self.trim_old_half()?;
        }
        let mut buf = String::with_capacity(line.len() + 1);
        buf.push_str(line);
        buf.push('\n');
        self.file.write_all(buf.as_bytes())?;
        self.size += buf.len() as u64;
        Ok(())
    }

    /// Dosyanın eski yarısını (satır sınırında) atar.
    fn trim_old_half(&mut self) -> io::Result<()> {
        let data = std::fs::read(&self.path)?;
        let mut start = data.len() / 2;
        // Satır başına ilerle.
        while start < data.len() && data[start - 1] != b'\n' {
            start += 1;
        }
        let tmp = self.path.with_extension("log.tmp");
        {
            let mut out = open_truncate(&tmp)?;
            out.write_all(&data[start.min(data.len())..])?;
        }
        std::fs::rename(&tmp, &self.path)?;
        self.file = open_append(&self.path)?;
        self.size = self.file.metadata()?.len();
        Ok(())
    }
}

fn open_append(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

fn open_truncate(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

fn line(ts: &str, mut fields: Map<String, Value>) -> String {
    fields.insert("ts".into(), Value::String(ts.to_owned()));
    Value::Object(fields).to_string()
}

fn writer(mut log: LogFile, rx: &std::sync::mpsc::Receiver<Msg>) {
    // Başarılı REST isteklerinin rota şablonu başına sayacı.
    let mut ok: BTreeMap<String, u64> = BTreeMap::new();
    let mut last_counts = Instant::now();
    loop {
        match rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Msg::Event { ts, fields }) => {
                if fields.get("kind").and_then(Value::as_str) == Some("rest_ok") {
                    // Rota şablonu zaten "METHOD /yol" biçiminde.
                    let route = fields.get("route").and_then(Value::as_str).unwrap_or("?");
                    *ok.entry(route.to_owned()).or_insert(0) += 1;
                } else {
                    let _ = log.append(&line(&ts, fields));
                }
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                flush_counts(&mut log, &mut ok);
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if last_counts.elapsed() >= COUNTS_EVERY {
            flush_counts(&mut log, &mut ok);
            last_counts = Instant::now();
        }
    }
}

fn flush_counts(log: &mut LogFile, ok: &mut BTreeMap<String, u64>) {
    if ok.is_empty() {
        return;
    }
    let mut fields = Map::new();
    fields.insert("kind".into(), "rest_counts".into());
    let counts: Map<String, Value> = std::mem::take(ok)
        .into_iter()
        .map(|(k, v)| (k, Value::from(v)))
        .collect();
    fields.insert("counts".into(), Value::Object(counts));
    let _ = log.append(&line(&Timestamp::now().to_string(), fields));
}

/// `:diag` ekranının satırları. `# ` ile başlayan satırlar başlıktır.
///
/// Diski okur (en çok `MAX_BYTES`): UI thread'inde çağrılmamalı.
pub fn report(path: &Path, recent: usize) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => summarize(&text, jiff::tz::TimeZone::system(), recent),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            vec!["Henüz teşhis kaydı yok.".into()]
        }
        Err(e) => vec![format!("Teşhis günlüğü okunamadı: {e}")],
    }
}

#[derive(Default)]
struct Counters {
    identify: u64,
    resume: u64,
    ready: u64,
    resumed: u64,
    gw_close: BTreeMap<u64, u64>,
    voice_close: BTreeMap<u64, u64>,
    op4: u64,
    throttled: u64,
    rest_ok: u64,
    rest_4xx: u64,
    rest_5xx: u64,
    rest_429: u64,
    captcha: u64,
    token_revoked: u64,
    voice_sessions: u64,
}

fn codes(map: &BTreeMap<u64, u64>) -> String {
    if map.is_empty() {
        return "yok".into();
    }
    map.iter()
        .map(|(c, n)| {
            if *c == 0 {
                format!("kodsuz×{n}")
            } else {
                format!("{c}×{n}")
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// Günlük metninden özet + son olaylar. `today`: yerel saat dilimindeki bugün.
fn summarize(text: &str, tz: jiff::tz::TimeZone, recent: usize) -> Vec<String> {
    let today = Timestamp::now().to_zoned(tz.clone()).date();
    let mut c = Counters::default();
    let mut events: Vec<String> = Vec::new();
    for raw in text.lines() {
        let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        let Some(kind) = obj.get("kind").and_then(Value::as_str) else {
            continue;
        };
        let Some(ts) = obj
            .get("ts")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<Timestamp>().ok())
        else {
            continue;
        };
        let zoned = ts.to_zoned(tz.clone());
        events.push(format_event(&zoned, kind, &obj));
        if zoned.date() != today {
            continue;
        }
        let code = obj.get("code").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "gw_identify" => c.identify += 1,
            "gw_resume" => c.resume += 1,
            "gw_ready" => c.ready += 1,
            "gw_resumed" => c.resumed += 1,
            "gw_close" => *c.gw_close.entry(code).or_insert(0) += 1,
            "voice_close" => *c.voice_close.entry(code).or_insert(0) += 1,
            "gw_op4" => c.op4 += 1,
            "gw_identify_throttled" => c.throttled += 1,
            "voice_session_start" => c.voice_sessions += 1,
            "rest_counts" => {
                if let Some(Value::Object(m)) = obj.get("counts") {
                    c.rest_ok += m.values().filter_map(Value::as_u64).sum::<u64>();
                }
            }
            "rest_err" => match obj.get("status").and_then(Value::as_u64) {
                Some(400..=499) => c.rest_4xx += 1,
                Some(500..=599) => c.rest_5xx += 1,
                _ => {}
            },
            "rest_429" => c.rest_429 += 1,
            "captcha" => c.captcha += 1,
            "token_revoked" => c.token_revoked += 1,
            _ => {}
        }
    }
    let mut out = vec![
        format!("# Özet (bugün, {today})"),
        format!("IDENTIFY: {}   RESUME: {}", c.identify, c.resume),
        format!("READY: {}   RESUMED: {}", c.ready, c.resumed),
        format!("Gateway kapanma kodları: {}", codes(&c.gw_close)),
        format!(
            "Ses oturumu: {}   op 4 gönderimi: {}   ses kapanma kodları: {}",
            c.voice_sessions,
            c.op4,
            codes(&c.voice_close)
        ),
        format!(
            "REST: 2xx≈{} (10 dk'da bir toplanır)   4xx: {}   5xx: {}   429: {}",
            c.rest_ok, c.rest_4xx, c.rest_5xx, c.rest_429
        ),
        format!(
            "Captcha istendi: {}   Token iptal: {}   IDENTIFY sınırı beklemesi: {}",
            c.captcha, c.token_revoked, c.throttled
        ),
        String::new(),
        format!("# Son {recent} olay (en yeni altta)"),
    ];
    let skip = events.len().saturating_sub(recent);
    out.extend(events.into_iter().skip(skip));
    out
}

fn format_event(at: &jiff::Zoned, kind: &str, obj: &Map<String, Value>) -> String {
    let mut s = format!("{} {kind}", at.strftime("%m-%d %H:%M:%S"));
    for (k, v) in obj {
        if matches!(k.as_str(), "kind" | "ts" | "counts") {
            continue;
        }
        match v {
            Value::String(t) => {
                let _ = write!(s, " {k}={t}");
            }
            other => {
                let _ = write!(s, " {k}={other}");
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    // Gerçek token değil; sadece biçimi taklit eden sahte değer.
    const FAKE: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAA.BBBBBB.CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("digcord-diag-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dizin");
        dir.join("diag.log")
    }

    #[test]
    fn token_and_secrets_never_reach_the_file() {
        let path = temp("secret");
        let (layer, guard) = init(path.clone()).expect("init");
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            // Yasaklı alan adları atılır; izinli alanlardaki token benzeri değerler maskelenir.
            tracing::info!(
                target: "diag",
                kind = "gw_close",
                code = 4004u64,
                token = FAKE,
                authorization = FAKE,
                cookie = "sid=abc",
                content = "gizli mesaj",
                username = "birisi",
                reason = %format!("sunucu kapattı: {FAKE}"),
            );
            // Başka hedefler hiç yazılmaz.
            tracing::info!(target: "baska", kind = "x", reason = FAKE);
            // `kind` olmayan olay atılır.
            tracing::info!(target: "diag", reason = "kind yok");
        });
        drop(guard);
        let text = std::fs::read_to_string(&path).expect("oku");
        assert!(!text.contains(FAKE), "{text}");
        for needle in ["sid=abc", "gizli mesaj", "birisi", "authorization", "baska"] {
            assert!(!text.contains(needle), "{needle} bulundu: {text}");
        }
        assert_eq!(text.lines().count(), 1, "{text}");
        let v: Value = serde_json::from_str(text.trim()).expect("json");
        assert_eq!(v["kind"], "gw_close");
        assert_eq!(v["code"], 4004);
        assert!(v["reason"].as_str().is_some_and(|r| r.contains("[TOKEN]")));
        assert!(v["ts"].is_string());
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&path).expect("meta").permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn rest_ok_is_counted_not_logged() {
        let path = temp("counts");
        let (layer, guard) = init(path.clone()).expect("init");
        let subscriber = tracing_subscriber::registry().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..3 {
                tracing::info!(target: "diag", kind = "rest_ok", method = "GET", route = "GET /channels/{id}/messages");
            }
            tracing::info!(target: "diag", kind = "rest_err", method = "GET", route = "GET /x", status = 403u64);
        });
        drop(guard);
        let text = std::fs::read_to_string(&path).expect("oku");
        assert_eq!(text.matches("rest_ok").count(), 0, "{text}");
        let counts: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).expect("json"))
            .filter(|v: &Value| v["kind"] == "rest_counts")
            .collect();
        assert_eq!(counts.len(), 1, "{text}");
        assert_eq!(counts[0]["counts"]["GET /channels/{id}/messages"], 3);
    }

    #[test]
    fn long_values_are_truncated() {
        let long = "ş".repeat(1000);
        let out = sanitize(&long);
        assert_eq!(out.chars().count(), VALUE_MAX + 1);
    }

    #[test]
    fn file_is_trimmed_to_newer_half_and_stays_private() {
        let path = temp("trim");
        let mut log = LogFile::open(path.clone(), 1000).expect("aç");
        for i in 0..200 {
            log.append(&format!("{{\"kind\":\"t\",\"i\":{i}}}"))
                .expect("yaz");
        }
        let text = std::fs::read_to_string(&path).expect("oku");
        assert!(text.len() as u64 <= 1000, "{}", text.len());
        let last: Value = serde_json::from_str(text.lines().last().expect("satır")).expect("json");
        assert_eq!(last["i"], 199, "en yeni satır korunmalı");
        // Her satır bütün (satır ortasında kesilmedi).
        for l in text.lines() {
            serde_json::from_str::<Value>(l).expect("bütün satır");
        }
        let mode = std::os::unix::fs::PermissionsExt::mode(
            &std::fs::metadata(&path).expect("meta").permissions(),
        );
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn summary_counts_today_and_lists_recent() {
        let now = Timestamp::now().to_string();
        let old = "2020-01-01T00:00:00Z";
        let log = [
            format!(r#"{{"ts":"{now}","kind":"gw_identify"}}"#),
            format!(r#"{{"ts":"{now}","kind":"gw_identify"}}"#),
            format!(r#"{{"ts":"{now}","kind":"gw_resume"}}"#),
            format!(r#"{{"ts":"{now}","kind":"gw_close","code":4003,"reason":"x"}}"#),
            format!(r#"{{"ts":"{now}","kind":"gw_close","code":4003,"reason":"x"}}"#),
            format!(r#"{{"ts":"{now}","kind":"gw_close","code":4004,"reason":"x"}}"#),
            format!(r#"{{"ts":"{now}","kind":"rest_err","status":403,"route":"GET /a"}}"#),
            format!(r#"{{"ts":"{now}","kind":"rest_err","status":500,"route":"GET /a"}}"#),
            format!(r#"{{"ts":"{now}","kind":"rest_429","route":"GET /a"}}"#),
            format!(r#"{{"ts":"{now}","kind":"rest_counts","counts":{{"GET /a":5,"GET /b":2}}}}"#),
            format!(r#"{{"ts":"{now}","kind":"captcha","status":400}}"#),
            format!(r#"{{"ts":"{old}","kind":"gw_identify"}}"#),
            "bozuk satır".to_owned(),
        ]
        .join("\n");
        let out = summarize(&log, jiff::tz::TimeZone::UTC, 3);
        let all = out.join("\n");
        assert!(all.contains("IDENTIFY: 2   RESUME: 1"), "{all}");
        assert!(all.contains("4003×2  4004×1"), "{all}");
        assert!(all.contains("2xx≈7"), "{all}");
        assert!(all.contains("4xx: 1   5xx: 1   429: 1"), "{all}");
        assert!(all.contains("Captcha istendi: 1"), "{all}");
        // Son 3 olay (bozuk satır sayılmaz), en yeni altta.
        let recent: Vec<_> = out
            .iter()
            .skip_while(|l| !l.starts_with("# Son"))
            .skip(1)
            .collect();
        assert_eq!(recent.len(), 3, "{out:?}");
        assert!(
            recent.last().is_some_and(|l| l.contains("gw_identify")),
            "{out:?}"
        );
    }
}
