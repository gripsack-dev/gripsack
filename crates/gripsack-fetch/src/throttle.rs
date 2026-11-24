//! Token-bucket rate budgets per domain (0002 §throttle).
//!
//! Budgets come from three sources, in increasing precedence:
//! built-in defaults (the internal fetchers' registries) < plugin-
//! declared (the `capabilities` op — rate budgets live in fetchers)
//! < env.toml `[throttle]`. Buckets persist across runs in
//! $GRIPSACK_HOME/throttle.json, so back-to-back applies share one
//! budget — that is the GitHub-403 failure mode this exists for.

use parking_lot::{Mutex, MutexGuard};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};

mod persistence;
pub mod rate_limit;

#[derive(Debug)]
struct DeadlineExpired;

/// Built-in budgets for the registries the internal fetchers call.
/// Downloads (release CDNs, tarball mirrors) are deliberately not
/// throttled — rate limits live on API endpoints.
///
/// `api.github.com` is auth-aware: authenticated clients get 5000/h
/// (about 83/min), so 30/min is safe; anonymous clients get 60/h, so
/// they are paced at 1/min with a small burst for a short session. The
/// server's own `x-ratelimit-*` headers refine either (see
/// [`Throttle::observe_quota`]).
fn defaults(github_authenticated: bool) -> [(&'static str, &'static str, u64); 1] {
    if github_authenticated {
        [("api.github.com", "30/min", 1)]
    } else {
        [("api.github.com", "1/min", ANONYMOUS_GITHUB_BURST)]
    }
}

/// Requests an anonymous client may make back to back before pacing.
const ANONYMOUS_GITHUB_BURST: u64 = 5;

/// Whether a GitHub token is bound (the same variables the http
/// client's credential policy reads).
fn github_authenticated_from_env() -> bool {
    ["GITHUB_TOKEN", "GH_TOKEN"].iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|token| !token.trim().is_empty())
    })
}

/// The server's last word on a domain's quota (`x-ratelimit-remaining`
/// and `-reset`). Authoritative over the local token estimate: at zero
/// no request is admitted before `reset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Quota {
    pub(crate) remaining: u64,
    pub(crate) reset: SystemTime,
}

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
    quota: Option<Quota>,
}

impl Bucket {
    fn new(budget: rate_limit::BinaryTokenRate) -> Self {
        Self::with_burst(budget, 1)
    }

    fn with_burst(budget: rate_limit::BinaryTokenRate, burst: u64) -> Self {
        Bucket {
            tokens: rate_limit::TokenBucket::full_with_burst(budget, burst),
            updated: SystemTime::now(),
            quota: None,
        }
    }

    /// Time until the server-reported quota admits a request; expired
    /// quotas are forgotten.
    fn quota_wait(&mut self, now: SystemTime) -> Option<Duration> {
        let quota = self.quota?;
        match quota.reset.duration_since(now) {
            Ok(wait) if quota.remaining == 0 && !wait.is_zero() => Some(wait),
            Ok(_) => None,
            Err(_) => {
                self.quota = None;
                None
            }
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
        Self::with_auth(overrides, persist, github_authenticated_from_env())
    }

    /// `new` with the GitHub credential state given instead of read
    /// from the environment.
    pub fn with_auth(
        overrides: &BTreeMap<String, String>,
        persist: Option<PathBuf>,
        github_authenticated: bool,
    ) -> Self {
        let mut buckets = BTreeMap::new();
        for (domain, budget, burst) in defaults(github_authenticated) {
            if let Some(budget) = parse_budget(budget) {
                buckets.insert(domain.to_string(), Bucket::with_burst(budget, burst));
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
        let mut deadline = deadline.map(gripsack_host::process::OperationDeadline::at);
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
        deadline: &mut Option<gripsack_host::process::OperationDeadline>,
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
        let mut deadline = deadline.map(gripsack_host::process::OperationDeadline::at);
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
                        let now = SystemTime::now();
                        bucket.refill(now);
                        if let Some(wait) = bucket.quota_wait(now) {
                            wait
                        } else {
                            match bucket.tokens.take() {
                                rate_limit::TokenAdmission::Granted => {
                                    if let Some(quota) = &mut bucket.quota {
                                        quota.remaining = quota.remaining.saturating_sub(1);
                                    }
                                    return ThrottleAdmission::Granted;
                                }
                                rate_limit::TokenAdmission::Wait { nanoseconds } => {
                                    Duration::from_nanos(nanoseconds)
                                }
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

    /// Learn a domain's real quota from a response's `x-ratelimit-*`
    /// headers. Domains without a bucket are ignored. The observation
    /// replaces any earlier one (the server counts every client sharing
    /// the address/token) and is persisted with the bucket.
    pub fn observe_quota(&self, domain: &str, remaining: u64, reset: SystemTime) {
        if let Some(bucket) = self.buckets.lock().get_mut(domain) {
            bucket.quota = Some(Quota { remaining, reset });
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
            let _ = gripsack_host::fs::atomic_write_at(path, &json);
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
    if gripsack_host::process::OperationDeadline::at(deadline)
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

/// Feed one response's rate-limit headers (`x-ratelimit-remaining`,
/// `x-ratelimit-reset` as epoch seconds) into the installed throttle.
pub(crate) fn observe_response(url: &str, remaining: Option<&str>, reset: Option<&str>) {
    let (Some(throttle), Some(host)) = (global(), url_host(url)) else {
        return;
    };
    if let Some((remaining, reset)) = parse_quota_headers(remaining, reset) {
        throttle.observe_quota(&host, remaining, reset);
    }
}

fn parse_quota_headers(remaining: Option<&str>, reset: Option<&str>) -> Option<(u64, SystemTime)> {
    let remaining = remaining?.trim().parse::<u64>().ok()?;
    let reset = reset?.trim().parse::<u64>().ok()?;
    let reset = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(reset))?;
    Some((remaining, reset))
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
    if gripsack_host::process::OperationDeadline::at(deadline)
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

    #[test]
    fn github_budget_is_auth_aware() {
        let take = |authenticated: bool| {
            let t = Throttle::with_auth(&BTreeMap::new(), None, authenticated);
            let mut buckets = t.buckets.lock();
            let bucket = buckets.get_mut("api.github.com").unwrap();
            let mut granted = 0;
            while bucket.tokens.take() == rate_limit::TokenAdmission::Granted {
                granted += 1;
            }
            let wait = match bucket.tokens.take() {
                rate_limit::TokenAdmission::Wait { nanoseconds } => nanoseconds,
                other => panic!("{other:?}"),
            };
            (granted, wait)
        };
        let (granted, wait) = take(false);
        assert_eq!(granted, ANONYMOUS_GITHUB_BURST, "anonymous burst");
        assert_eq!(wait, 60_000_000_000, "anonymous refill is 1/min");
        let (granted, wait) = take(true);
        assert_eq!(granted, 30);
        assert_eq!(wait, 2_000_000_000, "authenticated refill is 30/min");
    }

    #[test]
    fn a_burst_refills_at_the_rate_not_all_at_once() {
        let rate = parse_budget("1/min").unwrap();
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(4_000_000_000);
        let mut bucket = Bucket::with_burst(rate, 5);
        bucket.updated = start;
        while bucket.tokens.take() == rate_limit::TokenAdmission::Granted {}
        bucket.refill(start + Duration::from_secs(60));
        assert_eq!(bucket.tokens.take(), rate_limit::TokenAdmission::Granted);
        assert!(matches!(
            bucket.tokens.take(),
            rate_limit::TokenAdmission::Wait { .. }
        ));
    }

    #[test]
    fn quota_headers_parse_only_when_complete() {
        let (remaining, reset) = parse_quota_headers(Some("12"), Some("1800000000")).unwrap();
        assert_eq!(remaining, 12);
        assert_eq!(
            reset,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
        );
        assert!(parse_quota_headers(Some("12"), None).is_none());
        assert!(parse_quota_headers(None, Some("1")).is_none());
        assert!(parse_quota_headers(Some("x"), Some("1")).is_none());
    }

    #[test]
    fn an_exhausted_server_quota_blocks_until_reset_even_with_local_tokens() {
        let t = Throttle::with_auth(&BTreeMap::new(), None, true);
        t.observe_quota(
            "api.github.com",
            0,
            SystemTime::now() + Duration::from_secs(3600),
        );
        let deadline = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            t.acquire_before("api.github.com", Some(deadline)),
            ThrottleAdmission::DeadlineExpired
        );
        // an unrelated domain is unaffected, and an elapsed reset forgets the quota
        t.observe_quota(
            "api.github.com",
            0,
            SystemTime::now() - Duration::from_secs(1),
        );
        assert_eq!(
            t.acquire_before("api.github.com", Some(deadline)),
            ThrottleAdmission::Granted
        );
    }

    #[test]
    fn remaining_quota_counts_down_per_grant_and_unknown_domains_are_ignored() {
        let t = Throttle::with_auth(&BTreeMap::new(), None, true);
        t.observe_quota(
            "api.github.com",
            2,
            SystemTime::now() + Duration::from_secs(3600),
        );
        t.observe_quota("nowhere.example", 0, SystemTime::now());
        assert!(!t.buckets.lock().contains_key("nowhere.example"));
        let deadline = Instant::now() + Duration::from_secs(60);
        for _ in 0..2 {
            assert_eq!(
                t.acquire_before("api.github.com", Some(deadline)),
                ThrottleAdmission::Granted
            );
        }
        assert_eq!(
            t.acquire_before("api.github.com", Some(deadline)),
            ThrottleAdmission::DeadlineExpired
        );
    }

    #[test]
    fn learned_quota_persists_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("throttle.json");
        let reset = SystemTime::now() + Duration::from_secs(3600);
        let first = Throttle::with_auth(&BTreeMap::new(), Some(path.clone()), false);
        first.observe_quota("api.github.com", 0, reset);
        first.save();
        let second = Throttle::with_auth(&BTreeMap::new(), Some(path), false);
        let quota = second.buckets.lock()["api.github.com"].quota.unwrap();
        assert_eq!(quota.remaining, 0);
        let secs = |t: SystemTime| t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(secs(quota.reset), secs(reset));
        let deadline = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            second.acquire_before("api.github.com", Some(deadline)),
            ThrottleAdmission::DeadlineExpired
        );
    }
}
