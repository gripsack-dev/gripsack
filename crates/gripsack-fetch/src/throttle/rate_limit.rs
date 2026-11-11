//! Exact token-bucket transitions for every admitted finite binary rate.
//! Clock observations and persistent JSON admission remain caller boundaries;
//! credits, spending, wait ceilings and unit conversion use this implementation.
mod binary_rate;
mod credit_balance;
mod legacy_credits;
use binary_rate::IMPLICIT_SIGNIFICAND_BIT;
pub use binary_rate::{BinaryTokenRate, RatePeriod, admit_binary_rate};
pub use credit_balance::CREDIT_WORDS;
use credit_balance::{CreditBalance, CreditOrdering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenAdmission {
    Granted,
    Wait { nanoseconds: u64 },
}

fn deficit_wait(deficit: u128, credits_per_ns: u128) -> u64 {
    let quotient = (deficit - 1) / credits_per_ns;

    quotient as u64 + 1
}

fn capacity_for(rate: BinaryTokenRate) -> CreditBalance {
    let parts = rate.components();
    let period = parts.2.nanoseconds();
    let amount = (parts.0 as u128) * (period as u128);

    CreditBalance::from_shifted(amount, parts.1)
}

pub struct TokenBucket {
    rate: BinaryTokenRate,
    capacity: CreditBalance,
    available: CreditBalance,
}

impl TokenBucket {
    pub fn full(rate: BinaryTokenRate) -> Self {
        let cap = capacity_for(rate);
        let available = cap.copy();
        Self {
            rate,
            capacity: cap,
            available,
        }
    }

    pub fn declared_rate(&self) -> BinaryTokenRate {
        self.rate
    }

    pub fn credit_words(&self) -> &[u64; CREDIT_WORDS] {
        self.available.words()
    }

    pub fn restored(
        rate: BinaryTokenRate,
        saved_period: RatePeriod,
        words: [u64; CREDIT_WORDS],
    ) -> Self {
        let cap = capacity_for(rate);
        let mut available = CreditBalance::from_words(words);
        let parts = rate.components();
        available.rescale_capped(saved_period.nanoseconds(), parts.2.nanoseconds(), &cap);
        Self {
            rate,
            capacity: cap,
            available,
        }
    }

    pub fn from_legacy(rate: BinaryTokenRate, bits: u64) -> Option<Self> {
        let parts = rate.components();
        let mut available = legacy_credits::admit_legacy_credits(bits, parts.2)?;
        let cap = capacity_for(rate);
        available.clamp(&cap);
        Some(Self {
            rate,
            capacity: cap,
            available,
        })
    }

    pub fn reconfigure(&mut self, rate: BinaryTokenRate) {
        if self.rate == rate {
            return;
        }
        let previous = self.rate.components();
        let next = rate.components();
        let cap = capacity_for(rate);
        let mut available = self.available.copy();
        available.rescale_capped(previous.2.nanoseconds(), next.2.nanoseconds(), &cap);
        *self = Self {
            rate,
            capacity: cap,
            available,
        };
    }

    pub fn refill(&mut self, elapsed_ns: u128) {
        let parts = self.rate.components();
        let period = parts.2.nanoseconds();
        if elapsed_ns >= period as u128 {
            self.available = self.capacity.copy();
        } else {
            let elapsed = elapsed_ns as u64;
            let amount = (parts.0 as u128) * (elapsed as u128);
            let earned = CreditBalance::from_shifted(amount, parts.1);
            self.available.add_capped(&earned, &self.capacity);
        }
    }

    pub fn take(&mut self) -> TokenAdmission {
        let parts = self.rate.components();
        let period = parts.2.nanoseconds();
        let required = (IMPLICIT_SIGNIFICAND_BIT as u128) * (period as u128);
        let token = CreditBalance::from_small(required);
        if self.available.compare(&token) != CreditOrdering::Less {
            self.available.subtract_assign(&token);
            TokenAdmission::Granted
        } else {
            let available = self.available.to_small();
            let deficit = required - available;
            if parts.1 >= 42 {
                TokenAdmission::Wait { nanoseconds: 1 }
            } else {
                let shift = parts.1 as u64;
                let factor = 1u64 << shift;
                let per_ns = (parts.0 as u128) * (factor as u128);
                let nanoseconds = deficit_wait(deficit, per_ns);
                TokenAdmission::Wait { nanoseconds }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partial_refill_earns_the_declared_rate() {
        let rate = admit_binary_rate(2.0f64.to_bits(), RatePeriod::Second).unwrap();
        let mut bucket = TokenBucket::full(rate);
        assert_eq!(bucket.take(), TokenAdmission::Granted);
        assert_eq!(bucket.take(), TokenAdmission::Granted);
        assert!(matches!(bucket.take(), TokenAdmission::Wait { .. }));
        bucket.refill(500_000_000);
        assert_eq!(
            bucket.take(),
            TokenAdmission::Granted,
            "half a second at 2/s is one token"
        );
        assert!(matches!(bucket.take(), TokenAdmission::Wait { .. }));
    }
}
