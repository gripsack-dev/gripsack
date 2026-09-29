//! Exact-credit persistence with explicit migration from legacy floating tokens.
use super::Bucket;
use gripsack_policy::rate_limit::{BinaryTokenRate, CREDIT_WORDS, RatePeriod, TokenBucket};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CREDIT_FORMAT_VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedBucket<Credits> {
    version: u8,
    period_ns: u64,
    credits: Credits,
    updated_seconds: u64,
    updated_nanoseconds: u32,
}

fn period(nanoseconds: u64) -> Option<RatePeriod> {
    match nanoseconds {
        1_000_000_000 => Some(RatePeriod::Second),
        60_000_000_000 => Some(RatePeriod::Minute),
        3_600_000_000_000 => Some(RatePeriod::Hour),
        _ => None,
    }
}

fn timestamp(seconds: u64, nanoseconds: u32) -> Option<SystemTime> {
    if nanoseconds >= 1_000_000_000 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::new(seconds, nanoseconds))
}

fn decode(
    state: serde_json::Value,
    rate: BinaryTokenRate,
    now: SystemTime,
) -> Option<(TokenBucket, SystemTime)> {
    if state.get("version").is_some() {
        let saved: SavedBucket<[u64; CREDIT_WORDS]> = serde_json::from_value(state).ok()?;
        if saved.version != CREDIT_FORMAT_VERSION {
            return None;
        }
        let period = period(saved.period_ns)?;
        let tokens = TokenBucket::restored(rate, period, saved.credits);
        let updated = timestamp(saved.updated_seconds, saved.updated_nanoseconds).unwrap_or(now);
        Some((tokens, updated))
    } else {
        let tokens = match state.get("tokens").and_then(serde_json::Value::as_f64) {
            Some(tokens) => TokenBucket::from_legacy(rate, tokens.to_bits())?,
            None => TokenBucket::full(rate),
        };
        let updated = state
            .get("updated")
            .and_then(serde_json::Value::as_u64)
            .and_then(|seconds| timestamp(seconds, 0))
            .unwrap_or(now);
        Some((tokens, updated))
    }
}

pub(super) fn restore(text: &str, buckets: &mut BTreeMap<String, Bucket>) {
    let Ok(saved) = serde_json::from_str::<BTreeMap<String, serde_json::Value>>(text) else {
        return;
    };
    let now = SystemTime::now();
    for (domain, state) in saved {
        let Some(bucket) = buckets.get_mut(&domain) else {
            continue;
        };
        match decode(state, bucket.tokens.declared_rate(), now) {
            Some((tokens, updated)) => {
                bucket.tokens = tokens;
                bucket.updated = updated;
            }
            None => tracing::warn!(
                "ignoring invalid persisted throttle state for {}",
                gripsack_process::terminal::tame(domain),
            ),
        }
    }
}

struct SavedBuckets<'a>(&'a BTreeMap<String, Bucket>);

impl Serialize for SavedBuckets<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for (domain, bucket) in self.0 {
            let updated = bucket
                .updated
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let (_, _, period) = bucket.tokens.declared_rate().components();
            map.serialize_entry(
                domain,
                &SavedBucket {
                    version: CREDIT_FORMAT_VERSION,
                    period_ns: period.nanoseconds(),
                    credits: bucket.tokens.credit_words(),
                    updated_seconds: updated.as_secs(),
                    updated_nanoseconds: updated.subsec_nanos(),
                },
            )?;
        }
        map.end()
    }
}

pub(super) fn encode(buckets: &BTreeMap<String, Bucket>) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&SavedBuckets(buckets))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throttle::{Throttle, ThrottleAdmission};
    use gripsack_policy::rate_limit::TokenAdmission;
    use std::time::Instant;

    #[test]
    fn a_saved_fractional_timestamp_cannot_refill_an_already_accounted_interval() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("throttle.json");
        let overrides = BTreeMap::from([("persist.example".into(), "2/s".into())]);
        let observed = UNIX_EPOCH + Duration::new(4_000_000_000, 900_000_000);
        let throttle = Throttle::new(&overrides, Some(path.clone()));
        throttle
            .buckets
            .lock()
            .get_mut("persist.example")
            .unwrap()
            .updated = observed;
        let deadline = Instant::now() + Duration::from_secs(1);
        for _ in 0..2 {
            assert_eq!(
                throttle.acquire_before("persist.example", Some(deadline)),
                ThrottleAdmission::Granted
            );
        }
        throttle.save();

        let restored = Throttle::new(&overrides, Some(path));
        let mut buckets = restored.buckets.lock();
        let bucket = buckets.get_mut("persist.example").unwrap();
        bucket.refill(observed);
        assert_eq!(
            bucket.tokens.take(),
            TokenAdmission::Wait {
                nanoseconds: 500_000_000
            },
            "persisted_timestamp_refilled_consumed_interval",
        );
    }

    #[test]
    fn changed_period_preserves_a_saved_partial_token_instead_of_resetting_the_burst() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("throttle.json");
        let old = BTreeMap::from([("persist.example".into(), "1.5/hr".into())]);
        let observed = UNIX_EPOCH + Duration::from_secs(4_000_000_000);
        let throttle = Throttle::new(&old, Some(path.clone()));
        throttle
            .buckets
            .lock()
            .get_mut("persist.example")
            .unwrap()
            .updated = observed;
        let deadline = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            throttle.acquire_before("persist.example", Some(deadline)),
            ThrottleAdmission::Granted
        );
        throttle.save();

        let new = BTreeMap::from([("persist.example".into(), "1.5/min".into())]);
        let restored = Throttle::new(&new, Some(path));
        assert_eq!(
            restored.acquire_before("persist.example", Some(deadline)),
            ThrottleAdmission::DeadlineExpired,
            "persisted_period_reset_burst",
        );
        let mut buckets = restored.buckets.lock();
        let bucket = buckets.get_mut("persist.example").unwrap();
        assert_eq!(
            bucket.tokens.take(),
            TokenAdmission::Wait {
                nanoseconds: 20_000_000_000
            },
            "persisted_period_reset_burst",
        );
        bucket.refill(observed + Duration::from_secs(20));
        assert_eq!(bucket.tokens.take(), TokenAdmission::Granted);
    }
}
