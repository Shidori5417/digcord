//! Rota bazlı (bucket) rate limit yönetimi.
//!
//! Mantık (rota anahtarı, başlık ayrıştırma, durum güncelleme, bekleme hesabı) saf
//! fonksiyonlardır ve `now` dışarıdan verilir; G/Ç yok. Uyuma ve bucket başına sıralama
//! ([`RateLimiter::acquire`], [`RateLimiter::wait`]) ince bir kabuktur.
//!
//! Gözlem (PROTOCOL_NOTES §4.1): resmi istemcinin 200 yanıtlarında `x-ratelimit-*`
//! başlıkları görünmedi (CDP sınırlaması olabilir). Başlık yoksa bucket öğrenilmez ve
//! hiçbir şey engellenmez; yalnızca 429 yanıtları (gövde `retry_after` / `Retry-After`)
//! rota anahtarını geçici olarak tüketilmiş sayar.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use reqwest::header::HeaderMap;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::time::{Instant, sleep_until};

/// Taşmayı önlemek için bekleme üst sınırı.
const MAX_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

/// `METHOD /yol` biçiminde rota anahtarı. Büyük parametre (kanal/sunucu/webhook kimliği)
/// korunur, diğer kimlikler `:id`, tepki emojisi `:emoji` olur; sorgu dizesi atılır.
pub fn route_key(method: &str, path: &str) -> String {
    let path = path.split('?').next().unwrap_or("");
    let mut out = String::with_capacity(method.len() + path.len() + 1);
    out.push_str(method);
    out.push(' ');
    let mut prev = "";
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        if matches!(prev, "channels" | "guilds" | "webhooks") {
            out.push_str(seg);
        } else if prev == "reactions" {
            out.push_str(":emoji");
        } else if seg.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str(":id");
        } else {
            out.push_str(seg);
        }
        prev = seg;
    }
    out
}

/// Bir yanıttaki rate limit başlıkları. Eksik/bozuk alan `None`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Limits {
    pub bucket: Option<String>,
    pub remaining: Option<u32>,
    pub reset_after: Option<Duration>,
    pub global: bool,
    /// `Retry-After` başlığı (429 gövdesi yoksa yedek).
    pub retry_after: Option<Duration>,
}

impl Limits {
    pub fn from_headers(h: &HeaderMap) -> Self {
        let get = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::trim);
        Self {
            bucket: get("x-ratelimit-bucket")
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            remaining: get("x-ratelimit-remaining").and_then(|s| s.parse().ok()),
            reset_after: get("x-ratelimit-reset-after")
                .and_then(|s| s.parse().ok())
                .and_then(secs_to_duration),
            global: get("x-ratelimit-global").is_some_and(|s| s.eq_ignore_ascii_case("true")),
            retry_after: get("retry-after")
                .and_then(|s| s.parse().ok())
                .and_then(secs_to_duration),
        }
    }
}

/// Negatif/NaN/sonsuz değerleri reddeder, büyükleri sınırlar.
pub fn secs_to_duration(s: f64) -> Option<Duration> {
    if !s.is_finite() || s < 0.0 {
        return None;
    }
    Some(
        Duration::try_from_secs_f64(s)
            .unwrap_or(MAX_WAIT)
            .min(MAX_WAIT),
    )
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    remaining: u32,
    reset_at: Instant,
}

#[derive(Default)]
struct State {
    /// Rota anahtarı -> `X-RateLimit-Bucket` kimliği.
    routes: HashMap<String, String>,
    /// Bucket anahtarı (kimlik, bilinmiyorsa rota anahtarı) -> durum.
    buckets: HashMap<String, Bucket>,
    global_until: Option<Instant>,
}

impl State {
    fn bucket_key(&self, route: &str) -> String {
        self.routes
            .get(route)
            .map_or_else(|| route.to_owned(), Clone::clone)
    }
}

#[derive(Default)]
pub struct RateLimiter {
    state: Mutex<State>,
    /// Bucket anahtarı başına sıralama kilidi.
    slots: Mutex<HashMap<String, Arc<AsyncMutex<()>>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Aynı bucket'taki istekleri sıraya sokar; dönen koruyucu istek bitene kadar tutulmalı.
    pub async fn acquire(&self, route: &str) -> OwnedMutexGuard<()> {
        let key = lock(&self.state).bucket_key(route);
        let slot = lock(&self.slots).entry(key).or_default().clone();
        slot.lock_owned().await
    }

    /// Gönderimden önce beklenmesi gereken an (yoksa hemen gönder).
    pub fn wait_until(&self, route: &str, now: Instant) -> Option<Instant> {
        let st = lock(&self.state);
        let global = st.global_until.filter(|&t| t > now);
        let bucket = st
            .buckets
            .get(&st.bucket_key(route))
            .filter(|b| b.remaining == 0 && b.reset_at > now)
            .map(|b| b.reset_at);
        global.into_iter().chain(bucket).max()
    }

    /// Gerekirse uyur (global ve bucket).
    pub async fn wait(&self, route: &str) {
        if let Some(t) = self.wait_until(route, Instant::now()) {
            sleep_until(t).await;
        }
    }

    /// Yanıt başlıklarından bucket eşlemesini ve kalan hakkı günceller.
    /// `X-RateLimit-Bucket` yoksa hiçbir şey öğrenilmez (bkz. modül notu).
    pub fn record(&self, route: &str, limits: &Limits, now: Instant) {
        let Some(id) = &limits.bucket else { return };
        let mut st = lock(&self.state);
        st.routes.insert(route.to_owned(), id.clone());
        if let (Some(remaining), Some(reset_after)) = (limits.remaining, limits.reset_after) {
            st.buckets.insert(
                id.clone(),
                Bucket {
                    remaining,
                    reset_at: now + reset_after,
                },
            );
        }
    }

    /// 429: global ise herkesi, değilse yalnızca bu bucket'ı `wait` süresince engeller.
    pub fn record_429(&self, route: &str, wait: Duration, global: bool, now: Instant) {
        let until = now + wait.min(MAX_WAIT);
        let mut st = lock(&self.state);
        if global {
            st.global_until = Some(until);
        } else {
            let key = st.bucket_key(route);
            st.buckets.insert(
                key,
                Bucket {
                    remaining: 0,
                    reset_at: until,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
        }
        h
    }

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn route_keys() {
        assert_eq!(
            route_key("POST", "/channels/123/messages"),
            "POST /channels/123/messages"
        );
        assert_eq!(
            route_key("PATCH", "/channels/123/messages/456"),
            "PATCH /channels/123/messages/:id"
        );
        assert_eq!(
            route_key("DELETE", "/channels/123/messages/456"),
            "DELETE /channels/123/messages/:id"
        );
        assert_eq!(
            route_key(
                "PUT",
                "/channels/123/messages/456/reactions/%F0%9F%94%A5/@me?location=Message&type=0"
            ),
            "PUT /channels/123/messages/:id/reactions/:emoji/@me"
        );
        assert_eq!(
            route_key(
                "DELETE",
                "/channels/123/messages/456/reactions/a%3A9/0/@me?burst=false"
            ),
            "DELETE /channels/123/messages/:id/reactions/:emoji/:id/@me"
        );
        assert_eq!(
            route_key("GET", "/channels/123/messages?limit=10"),
            "GET /channels/123/messages"
        );
        assert_eq!(
            route_key("POST", "/channels/5/typing"),
            "POST /channels/5/typing"
        );
    }

    #[test]
    fn parses_headers() {
        let l = Limits::from_headers(&headers(&[
            ("x-ratelimit-bucket", "abc"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset-after", "1.5"),
            ("x-ratelimit-global", "true"),
            ("retry-after", "3"),
        ]));
        assert_eq!(l.bucket.as_deref(), Some("abc"));
        assert_eq!(l.remaining, Some(0));
        assert_eq!(l.reset_after, Some(MS(1500)));
        assert!(l.global);
        assert_eq!(l.retry_after, Some(Duration::from_secs(3)));
    }

    #[test]
    fn missing_or_bad_headers() {
        assert_eq!(Limits::from_headers(&HeaderMap::new()), Limits::default());
        let l = Limits::from_headers(&headers(&[
            ("x-ratelimit-remaining", "x"),
            ("x-ratelimit-reset-after", "-1"),
            ("retry-after", "NaN"),
        ]));
        assert_eq!(l, Limits::default());
        assert_eq!(secs_to_duration(f64::INFINITY), None);
        assert_eq!(secs_to_duration(1e300), Some(MAX_WAIT));
    }

    fn learn(rl: &RateLimiter, route: &str, remaining: u32, reset: Duration, now: Instant) {
        rl.record(
            route,
            &Limits {
                bucket: Some("b1".into()),
                remaining: Some(remaining),
                reset_after: Some(reset),
                ..Limits::default()
            },
            now,
        );
    }

    #[tokio::test(start_paused = true)]
    async fn wait_computation() {
        let rl = RateLimiter::new();
        let now = Instant::now();
        let r = "POST /channels/1/messages";
        assert_eq!(rl.wait_until(r, now), None, "bilinmeyen bucket engellemez");

        learn(&rl, r, 2, Duration::from_secs(1), now);
        assert_eq!(rl.wait_until(r, now), None, "hak varken beklemez");

        learn(&rl, r, 0, Duration::from_secs(2), now);
        assert_eq!(rl.wait_until(r, now), Some(now + Duration::from_secs(2)));
        // aynı bucket'a eşlenen başka rota da bekler
        let other = "PATCH /channels/1/messages/:id";
        learn(&rl, other, 0, Duration::from_secs(2), now);
        assert_eq!(
            rl.wait_until(other, now),
            Some(now + Duration::from_secs(2))
        );
        // süre dolunca engel kalkar
        let later = now + Duration::from_secs(2);
        assert_eq!(rl.wait_until(r, later), None);
    }

    #[tokio::test(start_paused = true)]
    async fn missing_headers_do_not_block() {
        let rl = RateLimiter::new();
        let now = Instant::now();
        rl.record("GET /x", &Limits::default(), now);
        assert_eq!(rl.wait_until("GET /x", now), None);
        // bucket var ama sayaç başlıkları yok: yine engellemez
        rl.record(
            "GET /x",
            &Limits {
                bucket: Some("b".into()),
                ..Limits::default()
            },
            now,
        );
        assert_eq!(rl.wait_until("GET /x", now), None);
    }

    #[tokio::test(start_paused = true)]
    async fn global_vs_bucket_429() {
        let rl = RateLimiter::new();
        let now = Instant::now();
        let a = "POST /channels/1/messages";
        let b = "POST /channels/2/messages";

        rl.record_429(a, Duration::from_secs(3), false, now);
        assert_eq!(rl.wait_until(a, now), Some(now + Duration::from_secs(3)));
        assert_eq!(rl.wait_until(b, now), None, "başka rota etkilenmez");

        rl.record_429(b, Duration::from_secs(5), true, now);
        assert_eq!(rl.wait_until(b, now), Some(now + Duration::from_secs(5)));
        assert_eq!(
            rl.wait_until("GET /baska", now),
            Some(now + Duration::from_secs(5)),
            "global herkesi engeller"
        );
        // iki engel varsa en geç olan
        assert_eq!(rl.wait_until(a, now), Some(now + Duration::from_secs(5)));
        assert_eq!(rl.wait_until(a, now + Duration::from_secs(5)), None);
    }

    #[tokio::test(start_paused = true)]
    async fn bucket_429_uses_learned_bucket() {
        let rl = RateLimiter::new();
        let now = Instant::now();
        learn(&rl, "PUT /r1", 5, Duration::from_secs(1), now);
        learn(&rl, "DELETE /r2", 5, Duration::from_secs(1), now);
        rl.record_429("PUT /r1", Duration::from_secs(4), false, now);
        // aynı bucket'a bağlı rota da etkilenir
        assert_eq!(
            rl.wait_until("DELETE /r2", now),
            Some(now + Duration::from_secs(4))
        );
    }

    #[tokio::test(start_paused = true)]
    async fn sleeps_until_reset() {
        let rl = RateLimiter::new();
        let start = Instant::now();
        learn(&rl, "POST /a", 0, Duration::from_secs(2), start);
        rl.wait("POST /a").await;
        assert_eq!(start.elapsed(), Duration::from_secs(2));
        rl.wait("POST /a").await; // artık engel yok
        assert_eq!(start.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn same_bucket_is_serialized() {
        let rl = Arc::new(RateLimiter::new());
        let start = Instant::now();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tasks = Vec::new();
        for i in 0..3u32 {
            let (rl, log) = (rl.clone(), log.clone());
            tasks.push(tokio::spawn(async move {
                let _slot = rl.acquire("POST /channels/1/messages").await;
                rl.wait("POST /channels/1/messages").await;
                lock(&log).push((i, start.elapsed()));
                // "istek" sürüyor; sonra hak tükenmiş yanıt gelir
                tokio::time::sleep(MS(100)).await;
                learn(
                    &rl,
                    "POST /channels/1/messages",
                    0,
                    Duration::from_secs(1),
                    Instant::now(),
                );
            }));
        }
        for t in tasks {
            t.await.expect("görev");
        }
        let log = lock(&log).clone();
        assert_eq!(log.len(), 3);
        // ilk istek hemen; sonrakiler önceki bitip sıfırlama bitene kadar sırayla
        assert_eq!(log[0].1, Duration::ZERO);
        assert_eq!(log[1].1, MS(1100));
        assert_eq!(log[2].1, MS(2200));
    }

    #[tokio::test(start_paused = true)]
    async fn different_buckets_run_in_parallel() {
        let rl = Arc::new(RateLimiter::new());
        let a = rl.acquire("POST /channels/1/messages").await;
        // başka rota kilitlenmeden alınabilmeli
        let b = tokio::time::timeout(MS(1), rl.acquire("POST /channels/2/messages")).await;
        assert!(b.is_ok());
        drop(a);
    }
}
