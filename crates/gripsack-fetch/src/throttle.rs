//! Token-bucket rate budgets per domain (0002 §throttle).
//!
//! Budgets come from three sources, in increasing precedence:
//! built-in defaults (the internal fetchers' registries) < plugin-
//! declared (the `capabilities` op — rate budgets live in fetchers)
//! < env.toml `[throttle]`. Buckets persist across runs in
//! $GRIPSACK_HOME/throttle.json, so back-to-back applies share one
//! budget — that is the GitHub-403 failure mode this exists for.

use gripsack_policy::rate_limit;
use parking_lot::{Mutex, MutexGuard};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

mod persistence;

#[derive(Debug)]
struct DeadlineExpired;

/// Built-in budgets for the registries the internal fetchers call.
/// Downloads (release CDNs, tarball mirrors) are deliberately not
/// throttled — rate limits live on API endpoints.
const DEFAULTS: &[(&str, &str)] = &[
    ("api.github.com", "30/min"),
    ("ghcr.io", "30/min"),
    ("formulae.brew.sh", "60/min"),
];

/// Admit "N/unit" once; units are seconds, minutes or hours.
fn parse_budget(s: &str) -> Option<rate_limit::BinaryTokenRate> {
    let (n, unit) = s.trim().split_once('/')?;
    let n: f64 = n.trim().parse().ok()?;
    let period = match unit.trim() {
        "s" | "sec" | "second" => rate_limit::RatePeriod::Second,
        "m" | "min" | "minute" => rate_limit::RatePeriod::Minute,
        "h" | "hr" | "hour" => rate_limit::RatePeriod::Hour,
        _ => return None,
    };
    rate_limit::admit_binary_rate(n.to_bits(), period)
}

struct Bucket {
    tokens: rate_limit::TokenBucket,
    updated: SystemTime,
}

impl Bucket {
    fn new(budget: rate_limit::BinaryTokenRate) -> Self {
        Bucket {
            tokens: rate_limit::TokenBucket::full(budget),
            updated: SystemTime::now(),
        }
    }

    fn refill(&mut self, now: SystemTime) {
        if let Ok(elapsed) = now.duration_since(self.updated) {
            self.tokens.refill(elapsed.as_nanos());
            self.updated = now;
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
        self.register_before(domain, budget, None)
            .expect("unbounded rate registration");
    }

    fn register_before(
        &self,
        domain: &str,
        budget: &str,
        deadline: Option<Instant>,
    ) -> Result<(), DeadlineExpired> {
        let mut deadline = deadline.map(gripsack_process::OperationDeadline::at);
        if deadline
            .as_mut()
            .is_some_and(|deadline| deadline.remaining().is_none())
        {
            return Err(DeadlineExpired);
        }
        if self.user_declared.contains(domain) {
            return Ok(());
        }
        if let Some(budget) = parse_budget(budget) {
            let mut buckets = self.lock_before(&mut deadline)?;
            match buckets.get_mut(domain) {
                Some(bucket) => bucket.tokens.reconfigure(budget),
                None => {
                    buckets.insert(domain.to_owned(), Bucket::new(budget));
                }
            }
        }
        Ok(())
    }

    fn lock_before(
        &self,
        deadline: &mut Option<gripsack_process::OperationDeadline>,
    ) -> Result<MutexGuard<'_, BTreeMap<String, Bucket>>, DeadlineExpired> {
        let Some(deadline) = deadline else {
            return Ok(self.buckets.lock());
        };
        let end = deadline.instant();
        let Some(buckets) = self.buckets.try_lock_until(end) else {
            deadline.stop();
            return Err(DeadlineExpired);
        };
        deadline.remaining().ok_or(DeadlineExpired)?;
        Ok(buckets)
    }

    /// Block until one token is available for `domain`; unknown
    /// domains are unthrottled.
    pub fn acquire(&self, domain: &str) {
        self.acquire_before(domain, None);
    }

    fn acquire_before(&self, domain: &str, deadline: Option<Instant>) -> ThrottleAdmission {
        let mut deadline = deadline.map(gripsack_process::OperationDeadline::at);
        loop {
            if deadline
                .as_mut()
                .is_some_and(|deadline| deadline.remaining().is_none())
            {
                return ThrottleAdmission::DeadlineExpired;
            }
            let wait = {
                let mut buckets = match self.lock_before(&mut deadline) {
                    Ok(buckets) => buckets,
                    Err(DeadlineExpired) => return ThrottleAdmission::DeadlineExpired,
                };
                match buckets.get_mut(domain) {
                    None => return ThrottleAdmission::Granted,
                    Some(bucket) => {
                        bucket.refill(SystemTime::now());
                        match bucket.tokens.take() {
                            rate_limit::TokenAdmission::Granted => {
                                return ThrottleAdmission::Granted;
                            }
                            rate_limit::TokenAdmission::Wait { nanoseconds } => {
                                Duration::from_nanos(nanoseconds)
                            }
                        }
                    }
                }
            };
            if deadline
                .as_mut()
                .is_some_and(|deadline| !deadline.admit_wait(wait))
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
        let mut buckets = self.buckets.lock();
        persistence::restore(&text, &mut buckets);
    }

    pub fn save(&self) {
        let Some(path) = &self.persist else {
            return;
        };
        let json = persistence::encode(&self.buckets.lock());
        // the store's atomic write (temp + fsync + rename): a hand-
        // rolled tmp+rename could persist a torn file on crash
        if let Ok(json) = json {
            let _ = gripsack_fs::atomic_write_at(path, &json);
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
    if gripsack_process::OperationDeadline::at(deadline)
        .remaining()
        .is_none()
    {
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
    if gripsack_process::OperationDeadline::at(deadline)
        .remaining()
        .is_none()
    {
        return ThrottleAdmission::DeadlineExpired;
    }
    match global() {
        Some(throttle) => {
            if throttle
                .register_before(domain, budget, Some(deadline))
                .is_err()
            {
                return ThrottleAdmission::DeadlineExpired;
            }
            throttle.acquire_before(domain, Some(deadline))
        }
        None => ThrottleAdmission::Granted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_admission_rejects_nonfinite_and_unfillable_buckets() {
        for rate in [
            "NaN/s",
            "inf/min",
            "-inf/hr",
            "0.5/s",
            "-2/s",
            "1e309/s",
            "nope",
            "0/s",
            "1/fortnight",
        ] {
            assert_eq!(parse_budget(rate), None, "invalid_rate_admitted: {rate}");
        }
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
    fn mutex_contention_cannot_outlive_or_grant_after_the_original_deadline() {
        let throttle = Throttle::new(&BTreeMap::new(), None);
        throttle.register("registered.example", "1/hr");
        for domain in ["unknown.example", "registered.example"] {
            std::thread::scope(|scope| {
                let held = throttle.buckets.lock();
                let (send, receive) = std::sync::mpsc::channel();
                let shared = &throttle;
                let worker = scope.spawn(move || {
                    let deadline = Instant::now() + Duration::from_millis(30);
                    send.send(shared.acquire_before(domain, Some(deadline)))
                        .unwrap();
                });
                let result = receive.recv_timeout(Duration::from_secs(2));
                drop(held);
                worker.join().unwrap();
                assert_eq!(
                    result.expect("deadline_blocked_by_throttle_lock"),
                    ThrottleAdmission::DeadlineExpired,
                );
            });
        }
    }

    #[test]
    fn capability_registration_consumes_the_same_contended_deadline() {
        let throttle = Throttle::new(&BTreeMap::new(), None);
        std::thread::scope(|scope| {
            let held = throttle.buckets.lock();
            let (send, receive) = std::sync::mpsc::channel();
            let shared = &throttle;
            let worker = scope.spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(30);
                send.send(
                    shared
                        .register_before("plugin.example", "2/s", Some(deadline))
                        .is_err(),
                )
                .unwrap();
            });
            let result = receive.recv_timeout(Duration::from_secs(2));
            drop(held);
            worker.join().unwrap();
            assert!(result.unwrap());
        });
        assert!(!throttle.buckets.lock().contains_key("plugin.example"));
    }

    #[test]
    fn fractional_declarations_retain_their_exact_refill_boundary() {
        for (declaration, period_ns) in [
            ("1.5/s", 1_000_000_000u64),
            ("1.5/min", 60_000_000_000),
            ("1.5/hr", 3_600_000_000_000),
        ] {
            let start = SystemTime::UNIX_EPOCH + Duration::from_secs(4_000_000_000);
            let mut bucket = Bucket::new(parse_budget(declaration).unwrap());
            bucket.updated = start;
            assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
            let wait = period_ns.div_ceil(3);
            assert_eq!(
                bucket.tokens.take(),
                rate_limit::TokenAdmission::Wait { nanoseconds: wait },
                "declared_period_was_changed",
            );
            bucket.refill(start + Duration::from_nanos(wait - 1));
            assert_eq!(
                bucket.tokens.take(),
                rate_limit::TokenAdmission::Wait { nanoseconds: 1 }
            );
            bucket.refill(start + Duration::from_nanos(wait));
            assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
        }
    }

    #[test]
    fn a_backward_wall_clock_does_not_refill_the_same_interval_twice() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(4_000_000_000);
        let mut bucket = Bucket::new(parse_budget("1/s").unwrap());
        bucket.updated = start;
        assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
        bucket.refill(start + Duration::from_millis(500));
        bucket.refill(start + Duration::from_millis(250));
        bucket.refill(start + Duration::from_millis(500));
        assert_eq!(
            bucket.tokens.take(),
            rate_limit::TokenAdmission::Wait {
                nanoseconds: 500_000_000
            },
            "wall_clock_interval_reused",
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
        let mut buckets = t.buckets.lock();
        let bucket = buckets.get_mut("api.github.com").unwrap();
        for _ in 0..2 {
            assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
        }
        assert_eq!(
            bucket.tokens.take(),
            rate_limit::TokenAdmission::Wait {
                nanoseconds: 500_000_000
            }
        );
    }

    #[test]
    fn plugin_declaration_replaces_builtin_default() {
        let t = Throttle::new(&BTreeMap::new(), None);
        t.register("api.github.com", "10/s");
        let mut buckets = t.buckets.lock();
        let bucket = buckets.get_mut("api.github.com").unwrap();
        for _ in 0..10 {
            assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
        }
        assert_eq!(
            bucket.tokens.take(),
            rate_limit::TokenAdmission::Wait {
                nanoseconds: 100_000_000
            }
        );
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
