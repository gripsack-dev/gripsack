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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Built-in budgets for the registries the internal fetchers call.
/// Downloads (release CDNs, tarball mirrors) are deliberately not
/// throttled — rate limits live on API endpoints.
const DEFAULTS: &[(&str, &str)] = &[
    ("api.github.com", "30/min"),
    ("ghcr.io", "30/min"),
    ("formulae.brew.sh", "60/min"),
];

/// A bucket can hold at least one token and refills at a finite positive rate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RateBudget {
    capacity: f64,
    per_second: f64,
}

/// Admit "N/unit" once; units are seconds, minutes or hours.
fn parse_budget(s: &str) -> Option<RateBudget> {
    let (n, unit) = s.trim().split_once('/')?;
    let n: f64 = n.trim().parse().ok()?;
    if !n.is_finite() || n < 1.0 {
        return None;
    }
    let secs = match unit.trim() {
        "s" | "sec" | "second" => 1.0,
        "m" | "min" | "minute" => 60.0,
        "h" | "hr" | "hour" => 3600.0,
        _ => return None,
    };
    Some(RateBudget {
        capacity: n,
        per_second: n / secs,
    })
}

struct Bucket {
    tokens: f64,
    budget: RateBudget,
    updated: SystemTime,
}

impl Bucket {
    fn new(budget: RateBudget) -> Self {
        Bucket {
            tokens: budget.capacity,
            budget,
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
        self.tokens = (self.tokens + elapsed * self.budget.per_second).min(self.budget.capacity);
        self.updated = now;
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            // Rounding a sub-nanosecond wait to zero must not mint a token.
            Duration::from_secs_f64((1.0 - self.tokens) / self.budget.per_second)
                .max(Duration::from_nanos(1))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThrottleAdmission {
    Granted,
    DeadlineExpired,
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
            if let Some(budget) = parse_budget(budget) {
                buckets.insert(domain.to_string(), Bucket::new(budget));
            }
        }
        let mut user_declared = BTreeSet::new();
        for (domain, budget) in overrides {
            match parse_budget(budget) {
                Some(budget) => {
                    user_declared.insert(domain.clone());
                    buckets.insert(domain.clone(), Bucket::new(budget));
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
        if let Some(budget) = parse_budget(budget) {
            let mut buckets = self.buckets.lock().expect("throttle mutex");
            match buckets.get_mut(domain) {
                Some(bucket) => {
                    bucket.budget = budget;
                    bucket.tokens = bucket.tokens.min(budget.capacity);
                }
                None => {
                    buckets.insert(domain.to_owned(), Bucket::new(budget));
                }
            }
        }
    }

    /// Block until one token is available for `domain`; unknown
    /// domains are unthrottled.
    pub fn acquire(&self, domain: &str) {
        self.acquire_before(domain, None);
    }

    fn acquire_before(&self, domain: &str, deadline: Option<Instant>) -> ThrottleAdmission {
        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return ThrottleAdmission::DeadlineExpired;
            }
            let wait = {
                let mut buckets = self.buckets.lock().expect("throttle mutex");
                match buckets.get_mut(domain) {
                    None => return ThrottleAdmission::Granted,
                    Some(bucket) => {
                        let wait = bucket.refill();
                        if wait.is_zero() {
                            bucket.tokens -= 1.0;
                            return ThrottleAdmission::Granted;
                        }
                        wait
                    }
                }
            };
            if deadline
                .is_some_and(|deadline| wait >= deadline.saturating_duration_since(Instant::now()))
            {
                return ThrottleAdmission::DeadlineExpired;
            }
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
                    .unwrap_or(bucket.budget.capacity)
                    .clamp(0.0, bucket.budget.capacity);
                bucket.updated = state
                    .get("updated")
                    .and_then(|time| time.as_u64())
                    .and_then(|seconds| UNIX_EPOCH.checked_add(Duration::from_secs(seconds)))
                    .unwrap_or_else(SystemTime::now);
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

pub(crate) fn acquire_url_until(url: &str, deadline: Instant) -> ThrottleAdmission {
    if Instant::now() >= deadline {
        return ThrottleAdmission::DeadlineExpired;
    }
    match (global(), url_host(url)) {
        (Some(throttle), Some(host)) => throttle.acquire_before(&host, Some(deadline)),
        _ => ThrottleAdmission::Granted,
    }
}

/// Persist bucket state for the next run (call at command end).
pub fn save_global() {
    if let Some(t) = global() {
        t.save();
    }
}

/// Register a fetcher-declared budget, then admit one invocation within the
/// caller's existing operation deadline, including when no throttle is installed.
pub(crate) fn acquire_declared_until(
    domain: &str,
    budget: &str,
    deadline: Instant,
) -> ThrottleAdmission {
    if Instant::now() >= deadline {
        return ThrottleAdmission::DeadlineExpired;
    }
    match global() {
        Some(throttle) => {
            throttle.register(domain, budget);
            throttle.acquire_before(domain, Some(deadline))
        }
        None => ThrottleAdmission::Granted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_budget_syntax() {
        assert_eq!(
            parse_budget("2/s"),
            Some(RateBudget {
                capacity: 2.0,
                per_second: 2.0
            })
        );
        assert_eq!(
            parse_budget("60/min"),
            Some(RateBudget {
                capacity: 60.0,
                per_second: 1.0
            })
        );
        assert_eq!(
            parse_budget("5000/hr"),
            Some(RateBudget {
                capacity: 5000.0,
                per_second: 5000.0 / 3600.0
            })
        );
        assert_eq!(parse_budget("nope"), None);
        assert_eq!(parse_budget("0/s"), None);
        assert_eq!(parse_budget("1/fortnight"), None);
    }

    #[test]
    fn rate_admission_rejects_nonfinite_and_unfillable_buckets() {
        for rate in ["NaN/s", "inf/min", "-inf/hr", "0.5/s", "-2/s", "1e309/s"] {
            assert_eq!(parse_budget(rate), None, "{rate}");
        }
        assert_eq!(
            parse_budget("1/hr"),
            Some(RateBudget {
                capacity: 1.0,
                per_second: 1.0 / 3600.0
            })
        );
    }

    #[test]
    fn token_wait_cannot_outlive_or_reset_an_operation_deadline() {
        let throttle = Throttle::new(&BTreeMap::new(), None);
        throttle.register("slow.example", "1/hr");
        let deadline = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            throttle.acquire_before("slow.example", Some(deadline)),
            ThrottleAdmission::Granted
        );
        assert_eq!(
            throttle.acquire_before("slow.example", Some(deadline)),
            ThrottleAdmission::DeadlineExpired
        );
        assert_eq!(
            throttle.acquire_before("unknown.example", Some(Instant::now())),
            ThrottleAdmission::DeadlineExpired
        );
    }

    #[test]
    fn persisted_token_and_time_bounds_cannot_panic_or_mint_extra_tokens() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("throttle.json");
        let overrides = BTreeMap::from([("bounded.example".into(), "1/hr".into())]);
        for tokens in [-1e308, 1e308] {
            std::fs::write(
                &path,
                serde_json::json!({
                    "bounded.example": {"tokens": tokens, "updated": u64::MAX}
                })
                .to_string(),
            )
            .unwrap();
            let throttle = Throttle::new(&overrides, Some(path.clone()));
            let deadline = Instant::now() + Duration::from_secs(60);
            assert_eq!(
                throttle.acquire_before("bounded.example", Some(deadline)),
                if tokens < 0.0 {
                    ThrottleAdmission::DeadlineExpired
                } else {
                    ThrottleAdmission::Granted
                }
            );
            assert_eq!(
                throttle.acquire_before("bounded.example", Some(deadline)),
                ThrottleAdmission::DeadlineExpired
            );
        }
    }

    #[test]
    fn user_override_beats_plugin_declaration() {
        let mut overrides = BTreeMap::new();
        overrides.insert("api.github.com".to_string(), "2/s".to_string());
        let t = Throttle::new(&overrides, None);
        t.register("api.github.com", "5000/hr");
        let buckets = t.buckets.lock().expect("mutex");
        assert_eq!(buckets["api.github.com"].budget.capacity, 2.0);
    }

    #[test]
    fn plugin_declaration_replaces_builtin_default() {
        let t = Throttle::new(&BTreeMap::new(), None);
        t.register("api.github.com", "10/s");
        let buckets = t.buckets.lock().expect("mutex");
        assert_eq!(buckets["api.github.com"].budget.capacity, 10.0);
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
