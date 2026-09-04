//! Token-bucket rate budgets per domain (0002 §throttle).
//!
//! Budgets come from three sources, in increasing precedence:
//! built-in defaults (the internal fetchers' registries) < plugin-
//! declared (the `capabilities` op — rate budgets live in fetchers)
//! < env.toml `[throttle]`. Buckets persist across runs in
//! $GRIPSACK_HOME/throttle.json, so back-to-back applies share one
//! budget — that is the GitHub-403 failure mode this exists for.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Built-in budgets for the registries the internal fetchers call.
/// Downloads (release CDNs, tarball mirrors) are deliberately not
/// throttled — rate limits live on API endpoints.
const DEFAULTS: &[(&str, &str)] = &[
    ("api.github.com", "30/min"),
    ("ghcr.io", "30/min"),
    ("formulae.brew.sh", "60/min"),
];

/// "N/unit" → (capacity, refill per second). Units: s, min, hr.
pub fn parse_budget(s: &str) -> Option<(f64, f64)> {
    let (n, unit) = s.trim().split_once('/')?;
    let n: f64 = n.trim().parse().ok()?;
    if n <= 0.0 {
        return None;
    }
    let secs = match unit.trim() {
        "s" | "sec" | "second" => 1.0,
        "m" | "min" | "minute" => 60.0,
        "h" | "hr" | "hour" => 3600.0,
        _ => return None,
    };
    Some((n, n / secs))
}

struct Bucket {
    tokens: f64,
    capacity: f64,
    per_sec: f64,
    updated: SystemTime,
}

impl Bucket {
    fn new(capacity: f64, per_sec: f64) -> Self {
        Bucket {
            tokens: capacity,
            capacity,
            per_sec,
            updated: SystemTime::now(),
        }
    }

    /// Refill from elapsed time; the wait until the next token.
    fn refill(&mut self) -> Duration {
        let now = SystemTime::now();
        let elapsed = now
            .duration_since(self.updated)
            .unwrap_or_default()
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        self.updated = now;
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64((1.0 - self.tokens) / self.per_sec)
        }
    }
}

pub struct Throttle {
    buckets: Mutex<BTreeMap<String, Bucket>>,
    /// Domains the user declared in env.toml — a plugin's capabilities
    /// must never lower these.
    user_declared: BTreeSet<String>,
    persist: Option<PathBuf>,
}

impl Throttle {
    pub fn new(overrides: &BTreeMap<String, String>, persist: Option<PathBuf>) -> Self {
        let mut buckets = BTreeMap::new();
        for (domain, budget) in DEFAULTS {
            if let Some((cap, rate)) = parse_budget(budget) {
                buckets.insert(domain.to_string(), Bucket::new(cap, rate));
            }
        }
        let mut user_declared = BTreeSet::new();
        for (domain, budget) in overrides {
            match parse_budget(budget) {
                Some((cap, rate)) => {
                    user_declared.insert(domain.clone());
                    buckets.insert(domain.clone(), Bucket::new(cap, rate));
                }
                None => {
                    tracing::warn!("ignoring unparseable [throttle] budget {domain} = {budget:?}")
                }
            }
        }
        let throttle = Throttle {
            buckets: Mutex::new(buckets),
            user_declared,
            persist,
        };
        throttle.load();
        throttle
    }

    /// A budget declared by a fetcher (capabilities op). Replaces a
    /// built-in default — the fetcher knows its registry best — but
    /// never a user declaration.
    pub fn register(&self, domain: &str, budget: &str) {
        if self.user_declared.contains(domain) {
            return;
        }
        if let Some((cap, rate)) = parse_budget(budget) {
            self.buckets
                .lock()
                .expect("throttle mutex")
                .entry(domain.to_string())
                .and_modify(|b| {
                    b.capacity = cap;
                    b.per_sec = rate;
                    b.tokens = b.tokens.min(cap);
                })
                .or_insert_with(|| Bucket::new(cap, rate));
        }
    }

    /// Block until one token is available for `domain`; unknown
    /// domains are unthrottled.
    pub fn acquire(&self, domain: &str) {
        loop {
            let wait = {
                let mut buckets = self.buckets.lock().expect("throttle mutex");
                match buckets.get_mut(domain) {
                    None => return,
                    Some(bucket) => {
                        let wait = bucket.refill();
                        if wait.is_zero() {
                            bucket.tokens -= 1.0;
                            return;
                        }
                        wait
                    }
                }
            };
            std::thread::sleep(wait);
        }
    }

    /// Throttle by URL host (the http choke point calls this).
    pub fn acquire_url(&self, url: &str) {
        if let Some(host) = url_host(url) {
            self.acquire(&host);
        }
    }

    fn load(&self) {
        let Some(path) = &self.persist else { return };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(saved) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(&text) else {
            return;
        };
        let mut buckets = self.buckets.lock().expect("throttle mutex");
        for (domain, state) in saved {
            if let Some(bucket) = buckets.get_mut(&domain) {
                bucket.tokens = state
                    .get("tokens")
                    .and_then(|t| t.as_f64())
                    .unwrap_or(bucket.capacity)
                    .min(bucket.capacity);
                bucket.updated = UNIX_EPOCH
                    + Duration::from_secs(
                        state
                            .get("updated")
                            .and_then(|t| t.as_u64())
                            .unwrap_or_else(|| {
                                SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs()
                            }),
                    );
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = &self.persist else {
            return;
        };
        let saved: BTreeMap<String, serde_json::Value> = self
            .buckets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(domain, b)| {
                (
                    domain.clone(),
                    serde_json::json!({
                        "tokens": b.tokens,
                        "updated": b.updated.duration_since(UNIX_EPOCH)
                            .unwrap_or_default().as_secs(),
                    }),
                )
            })
            .collect();
        // the store's atomic write (temp + fsync + rename): a hand-
        // rolled tmp+rename could persist a torn file on crash
        if let Ok(json) = serde_json::to_string(&saved) {
            let _ = gripsack_fs::atomic_write_at(path, json.as_bytes());
        }
    }
}

/// scheme://[user@]host[:port][/...] → lowercase host. Delegates to
/// the http crate's IPv6-aware parser — one URL grammar, not two
/// (the local one used to chop `[::1]:8443` to `[`).
fn url_host(url: &str) -> Option<String> {
    crate::http::host_port(url).map(|(host, _)| host)
}

static GLOBAL: OnceLock<Throttle> = OnceLock::new();

/// Install the process-wide throttle (idempotent — the first install
/// wins; commands in one process share it).
pub fn install(
    overrides: &BTreeMap<String, String>,
    persist: Option<PathBuf>,
) -> &'static Throttle {
    GLOBAL.get_or_init(|| Throttle::new(overrides, persist))
}

pub fn global() -> Option<&'static Throttle> {
    GLOBAL.get()
}

/// The http choke point: throttle `url`'s host if a throttle is
/// installed; a no-op otherwise (fetch used standalone, tests).
pub fn acquire_url(url: &str) {
    if let Some(t) = global() {
        t.acquire_url(url);
    }
}

/// Persist bucket state for the next run (call at command end).
pub fn save_global() {
    if let Some(t) = global() {
        t.save();
    }
}

/// A fetcher-declared budget (capabilities op): register, then take
/// a token for the invocation.
pub fn acquire_declared(domain: &str, budget: &str) {
    if let Some(t) = global() {
        t.register(domain, budget);
        t.acquire(domain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_budget_syntax() {
        assert_eq!(parse_budget("2/s"), Some((2.0, 2.0)));
        assert_eq!(parse_budget("60/min"), Some((60.0, 1.0)));
        assert_eq!(parse_budget("5000/hr"), Some((5000.0, 5000.0 / 3600.0)));
        assert_eq!(parse_budget("nope"), None);
        assert_eq!(parse_budget("0/s"), None);
        assert_eq!(parse_budget("1/fortnight"), None);
    }

    #[test]
    fn user_override_beats_plugin_declaration() {
        let mut overrides = BTreeMap::new();
        overrides.insert("api.github.com".to_string(), "2/s".to_string());
        let t = Throttle::new(&overrides, None);
        t.register("api.github.com", "5000/hr");
        let buckets = t.buckets.lock().expect("mutex");
        assert_eq!(buckets["api.github.com"].capacity, 2.0);
    }

    #[test]
    fn plugin_declaration_replaces_builtin_default() {
        let t = Throttle::new(&BTreeMap::new(), None);
        t.register("api.github.com", "10/s");
        let buckets = t.buckets.lock().expect("mutex");
        assert_eq!(buckets["api.github.com"].capacity, 10.0);
    }

    #[test]
    fn bucket_enforces_the_budget() {
        let t = Throttle::new(&BTreeMap::new(), None);
        t.register("test.local", "20/s");
        let start = std::time::Instant::now();
        for _ in 0..25 {
            t.acquire("test.local");
        }
        // 20 tokens burst-free; the 21st waits ~50ms, 25th ~250ms
        assert!(start.elapsed() >= Duration::from_millis(200));
    }

    #[test]
    fn unknown_domains_pass_through() {
        let t = Throttle::new(&BTreeMap::new(), None);
        t.acquire("anything.example"); // must not block or panic
    }

    #[test]
    fn url_host_extraction() {
        assert_eq!(
            url_host("https://api.github.com/repos/x/y"),
            Some("api.github.com".into())
        );
        assert_eq!(
            url_host("https://USER@ghcr.io:443/token"),
            Some("ghcr.io".into())
        );
        assert_eq!(url_host("not a url"), None);
    }
}
