//! Exact token-bucket transitions for every admitted finite binary rate.
//! Clock observations and persistent JSON admission remain caller boundaries;
//! credits, spending, wait ceilings and unit conversion use this implementation.
use vstd::prelude::*;
mod binary_rate;
mod credit_balance;
mod legacy_credits;
use binary_rate::IMPLICIT_SIGNIFICAND_BIT;
#[cfg(verus_keep_ghost)]
use binary_rate::SIGNIFICAND_LIMIT;
pub use binary_rate::{BinaryTokenRate, RatePeriod, admit_binary_rate};
pub use credit_balance::CREDIT_WORDS;
use credit_balance::{CreditBalance, CreditOrdering};
#[cfg(verus_keep_ghost)]
use credit_balance::{place, place_is_power};
#[cfg(verus_keep_ghost)]
use vstd::arithmetic::power2::{
    lemma_pow2_adds, lemma_pow2_pos, lemma_pow2_strictly_increases, lemma2_to64, lemma2_to64_rest,
    pow2,
};
#[cfg(verus_keep_ghost)]
use vstd::bits::{lemma_u64_pow2_no_overflow, lemma_u64_shl_is_mul};

verus! {

pub open spec fn rate_numerator(rate: BinaryTokenRate) -> nat {
    (rate.significand() as nat) * pow2(rate.shift() as nat)
}

pub open spec fn capacity(rate: BinaryTokenRate) -> nat {
    rate_numerator(rate) * (rate.period().nanos() as nat)
}

pub open spec fn one_token(rate: BinaryTokenRate) -> nat {
    (IMPLICIT_SIGNIFICAND_BIT as nat) * (rate.period().nanos() as nat)
}

proof fn rate_bounds(rate: BinaryTokenRate)
    requires rate.admitted(),
    ensures
        rate_numerator(rate) >= IMPLICIT_SIGNIFICAND_BIT,
        one_token(rate) <= capacity(rate),
        capacity(rate) < pow2(1118),
        2 * capacity(rate) < place(CREDIT_WORDS as nat),
{
    lemma_pow2_pos(rate.shift() as nat);
    if rate.shift() < 1023 {
        lemma_pow2_strictly_increases(rate.shift() as nat, 1023);
    }
    lemma2_to64();
    lemma2_to64_rest();
    lemma_pow2_adds(53, 1023);
    lemma_pow2_adds(1076, 42);
    lemma_pow2_adds(1, 1118);
    lemma_pow2_strictly_increases(1119, 1152);
    place_is_power(CREDIT_WORDS as nat);
    assert(rate_numerator(rate) >= IMPLICIT_SIGNIFICAND_BIT) by (nonlinear_arith)
        requires rate.significand() >= IMPLICIT_SIGNIFICAND_BIT, pow2(rate.shift() as nat) >= 1;
    assert(one_token(rate) <= capacity(rate)) by (nonlinear_arith)
        requires rate_numerator(rate) >= IMPLICIT_SIGNIFICAND_BIT;
    assert(capacity(rate) < pow2(1118)) by (nonlinear_arith)
        requires
            rate.significand() < SIGNIFICAND_LIMIT,
            pow2(53) == SIGNIFICAND_LIMIT,
            0 < pow2(rate.shift() as nat) <= pow2(1023),
            0 < rate.period().nanos() < pow2(42),
            pow2(1076) == pow2(53) * pow2(1023),
            pow2(1118) == pow2(1076) * pow2(42);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum TokenAdmission {
    Granted,
    Wait { nanoseconds: u64 },
}

fn deficit_wait(deficit: u128, credits_per_ns: u128, Ghost(period_ns): Ghost<u64>) -> (wait: u64)
    requires
        deficit > 0,
        credits_per_ns >= IMPLICIT_SIGNIFICAND_BIT,
        1 <= period_ns <= 3600000000000,
        deficit <= (period_ns as nat) * (IMPLICIT_SIGNIFICAND_BIT as nat),
    ensures
        1 <= wait <= period_ns,
        (wait as nat) * (credits_per_ns as nat) >= deficit,
        ((wait - 1) as nat) * (credits_per_ns as nat) < deficit,
{
    let quotient = (deficit - 1) / credits_per_ns;
    let ghost remainder = ((deficit as int) - 1) % (credits_per_ns as int);
    assert((deficit as int) - 1 == (quotient as int) * (credits_per_ns as int) + remainder)
        by (nonlinear_arith)
        requires
            quotient == ((deficit as int) - 1) / (credits_per_ns as int),
            remainder == ((deficit as int) - 1) % (credits_per_ns as int),
            credits_per_ns > 0, deficit > 0;
    assert(0 <= remainder < credits_per_ns);
    assert(quotient < period_ns) by (nonlinear_arith)
        requires
            (quotient as nat) * (credits_per_ns as nat) <= (deficit - 1) as nat,
            credits_per_ns >= IMPLICIT_SIGNIFICAND_BIT,
            deficit <= (period_ns as nat) * (IMPLICIT_SIGNIFICAND_BIT as nat),
            deficit > 0;
    let wait = quotient as u64 + 1;
    assert((wait as int) * (credits_per_ns as int) >= deficit as int) by (nonlinear_arith)
        requires
            (wait as int) == (quotient as int) + 1,
            (deficit as int) - 1 == (quotient as int) * (credits_per_ns as int) + remainder,
            0 <= remainder < credits_per_ns as int;
    wait
}

fn capacity_for(rate: BinaryTokenRate) -> (cap: CreditBalance)
    ensures cap.view() == capacity(rate), rate.admitted(),
{
    let parts = rate.components();
    let period = parts.2.nanoseconds();
    assert((parts.0 as int) * (period as int) <= u128::MAX as int) by (nonlinear_arith);
    let amount = (parts.0 as u128) * (period as u128);
    let cap = CreditBalance::from_shifted(amount, parts.1);
    assert(cap.view() == capacity(rate)) by (nonlinear_arith)
        requires
            cap.view() == (amount as nat) * pow2(parts.1 as nat),
            amount == (parts.0 as u128) * (period as u128),
            parts == (rate.significand(), rate.shift(), rate.period()),
            period == rate.period().nanos();
    cap
}

pub struct TokenBucket {
    rate: BinaryTokenRate,
    capacity: CreditBalance,
    available: CreditBalance,
}

impl TokenBucket {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        self.rate.admitted()
            && self.capacity.view() == capacity(self.rate)
            && self.available.view() <= self.capacity.view()
    }

    pub closed spec fn rate(&self) -> BinaryTokenRate { self.rate }
    pub closed spec fn available(&self) -> nat { self.available.view() }

    pub fn full(rate: BinaryTokenRate) -> (bucket: Self)
        ensures bucket.rate() == rate, bucket.available() == capacity(rate),
    {
        let cap = capacity_for(rate);
        let available = cap.copy();
        Self { rate, capacity: cap, available }
    }

    pub fn declared_rate(&self) -> (rate: BinaryTokenRate)
        ensures rate == self.rate(),
    {
        self.rate
    }

    pub fn credit_words(&self) -> (words: &[u64; CREDIT_WORDS])
        ensures credit_balance::prefix_value(words@, CREDIT_WORDS as nat) == self.available(),
    {
        self.available.words()
    }

    pub fn restored(rate: BinaryTokenRate, saved_period: RatePeriod, words: [u64; CREDIT_WORDS]) -> (bucket: Self)
        ensures
            bucket.rate() == rate,
            bucket.available() == if credit_balance::prefix_value(words@, CREDIT_WORDS as nat)
                * (rate.period().nanos() as nat) / (saved_period.nanos() as nat) >= capacity(rate) {
                    capacity(rate)
                } else {
                    credit_balance::prefix_value(words@, CREDIT_WORDS as nat)
                        * (rate.period().nanos() as nat) / (saved_period.nanos() as nat)
                },
    {
        let cap = capacity_for(rate);
        let mut available = CreditBalance::from_words(words);
        let parts = rate.components();
        available.rescale_capped(saved_period.nanoseconds(), parts.2.nanoseconds(), &cap);
        Self { rate, capacity: cap, available }
    }

    pub fn from_legacy(rate: BinaryTokenRate, bits: u64) -> (bucket: Option<Self>)
        ensures
            bucket.is_some() <==> legacy_credits::exponent(bits) <= binary_rate::MAX_FINITE_EXPONENT,
            bucket.is_some() ==> bucket.unwrap().rate() == rate,
            bucket.is_some() ==> bucket.unwrap().available() == if legacy_credits::legacy_amount(bits, rate.period()) > capacity(rate) {
                capacity(rate)
            } else {
                legacy_credits::legacy_amount(bits, rate.period())
            },
    {
        let parts = rate.components();
        let mut available = legacy_credits::admit_legacy_credits(bits, parts.2)?;
        let cap = capacity_for(rate);
        available.clamp(&cap);
        Some(Self { rate, capacity: cap, available })
    }

    pub fn reconfigure(&mut self, rate: BinaryTokenRate)
        ensures
            final(self).rate() == rate,
            final(self).available() == if old(self).available()
                * (rate.period().nanos() as nat) / (old(self).rate().period().nanos() as nat) >= capacity(rate) {
                    capacity(rate)
                } else {
                    old(self).available() * (rate.period().nanos() as nat) / (old(self).rate().period().nanos() as nat)
                },
    {
        proof { use_type_invariant(&*self); }
        if self.rate == rate {
            assert(self.available() * (rate.period().nanos() as nat) / (rate.period().nanos() as nat) == self.available())
                by (nonlinear_arith)
                requires rate.period().nanos() > 0;
            return;
        }
        let previous = self.rate.components();
        let next = rate.components();
        let cap = capacity_for(rate);
        let mut available = self.available.copy();
        available.rescale_capped(previous.2.nanoseconds(), next.2.nanoseconds(), &cap);
        *self = Self { rate, capacity: cap, available };
    }

    pub fn refill(&mut self, elapsed_ns: u128)
        ensures
            final(self).rate() == old(self).rate(),
            final(self).available() == if old(self).available() + rate_numerator(old(self).rate()) * (elapsed_ns as nat)
                >= capacity(old(self).rate()) {
                    capacity(old(self).rate())
                } else {
                    old(self).available() + rate_numerator(old(self).rate()) * (elapsed_ns as nat)
                },
    {
        proof { use_type_invariant(&*self); rate_bounds(self.rate); }
        let parts = self.rate.components();
        let period = parts.2.nanoseconds();
        if elapsed_ns >= period as u128 {
            proof {
                assert(self.available.view() + rate_numerator(self.rate) * (elapsed_ns as nat) >= capacity(self.rate))
                    by (nonlinear_arith)
                    requires elapsed_ns >= period, period == self.rate.period().nanos();
            }
            self.available = self.capacity.copy();
        } else {
            let elapsed = elapsed_ns as u64;
            assert((parts.0 as int) * (elapsed as int) <= u128::MAX as int) by (nonlinear_arith);
            let amount = (parts.0 as u128) * (elapsed as u128);
            let earned = CreditBalance::from_shifted(amount, parts.1);
            proof {
                assert(earned.view() == rate_numerator(self.rate) * (elapsed_ns as nat))
                    by (nonlinear_arith)
                    requires
                        earned.view() == (amount as nat) * pow2(parts.1 as nat),
                        amount == (parts.0 as u128) * (elapsed as u128),
                        elapsed == elapsed_ns,
                        parts == (self.rate.significand(), self.rate.shift(), self.rate.period());
            }
            self.available.add_capped(&earned, &self.capacity);
        }
    }

    pub fn take(&mut self) -> (admission: TokenAdmission)
        ensures
            final(self).rate() == old(self).rate(),
            (admission == TokenAdmission::Granted) <==> old(self).available() >= one_token(old(self).rate()),
            admission == TokenAdmission::Granted ==> final(self).available() == old(self).available() - one_token(old(self).rate()),
            match admission {
                TokenAdmission::Granted => true,
                TokenAdmission::Wait { nanoseconds } =>
                    final(self).available() == old(self).available()
                    && 1 <= nanoseconds <= old(self).rate().period().nanos()
                    && old(self).available() + rate_numerator(old(self).rate()) * (nanoseconds as nat) >= one_token(old(self).rate())
                    && old(self).available() + rate_numerator(old(self).rate()) * ((nanoseconds - 1) as nat) < one_token(old(self).rate()),
            },
    {
        proof { use_type_invariant(&*self); rate_bounds(self.rate); }
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
                proof {
                    lemma2_to64_rest();
                    if parts.1 > 42 {
                        lemma_pow2_strictly_increases(42, parts.1 as nat);
                    }
                    assert(rate_numerator(self.rate) >= one_token(self.rate)) by (nonlinear_arith)
                        requires
                            parts.0 >= IMPLICIT_SIGNIFICAND_BIT,
                            parts.0 == self.rate.significand(), parts.1 == self.rate.shift(),
                            period == self.rate.period().nanos(),
                            period < pow2(42),
                            pow2(42) <= pow2(parts.1 as nat);
                }
                assert(self.available() == old(self).available());
                assert(self.rate() == old(self).rate());
                assert(1 <= old(self).rate().period().nanos());
                assert(old(self).available() + rate_numerator(old(self).rate()) * 1 >= one_token(old(self).rate()));
                assert(old(self).available() + rate_numerator(old(self).rate()) * 0 < one_token(old(self).rate()));
                TokenAdmission::Wait { nanoseconds: 1 }
            } else {
                let shift = parts.1 as u64;
                proof {
                    lemma_u64_pow2_no_overflow(shift as nat);
                    lemma_u64_shl_is_mul(1, shift);
                }
                let factor = 1u64 << shift;
                assert((parts.0 as int) * (factor as int) <= u128::MAX as int) by (nonlinear_arith);
                let per_ns = (parts.0 as u128) * (factor as u128);
                proof {
                    assert(per_ns == rate_numerator(self.rate));
                }
                let nanoseconds = deficit_wait(deficit, per_ns, Ghost(period));
                assert(self.available.view() + rate_numerator(self.rate) * (nanoseconds as nat) >= one_token(self.rate))
                    by (nonlinear_arith)
                    requires
                        self.available.view() == available,
                        required == one_token(self.rate),
                        deficit == required - available,
                        per_ns == rate_numerator(self.rate),
                        (nanoseconds as nat) * (per_ns as nat) >= deficit;
                assert(self.available.view() + rate_numerator(self.rate) * ((nanoseconds - 1) as nat) < one_token(self.rate))
                    by (nonlinear_arith)
                    requires
                        self.available.view() == available,
                        required == one_token(self.rate),
                        deficit == required - available,
                        per_ns == rate_numerator(self.rate),
                        ((nanoseconds - 1) as nat) * (per_ns as nat) < deficit;
                assert(self.available() == old(self).available());
                assert(self.rate() == old(self).rate());
                assert(1 <= nanoseconds <= old(self).rate().period().nanos());
                assert(old(self).available() + rate_numerator(old(self).rate()) * (nanoseconds as nat) >= one_token(old(self).rate()));
                assert(old(self).available() + rate_numerator(old(self).rate()) * ((nanoseconds - 1) as nat) < one_token(old(self).rate()));
                TokenAdmission::Wait { nanoseconds }
            }
        }
    }
}

}
