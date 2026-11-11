//! Allocation-free credit arithmetic; eighteen words cover the admitted rate domain.
mod credit_words;
use credit_words::{WORD_BASE, add_word};

// Every accepted finite f64 rate >= 1 is m * 2^e / 2^52,
// with 2^52 <= m < 2^53 and 0 <= e <= 1023. Multiplication
// by the longest supported period (< 2^42 nanoseconds) needs
// fewer than 1118 bits. Eighteen 64-bit limbs leave headroom
// for adding two bounded balances without overflow.
pub const CREDIT_WORDS: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditOrdering {
    Less,
    Equal,
    Greater,
}

pub struct CreditBalance {
    words: [u64; CREDIT_WORDS],
}

impl CreditBalance {
    pub fn words(&self) -> &[u64; CREDIT_WORDS] {
        &self.words
    }

    pub fn from_words(words: [u64; CREDIT_WORDS]) -> Self {
        Self { words }
    }

    pub fn copy(&self) -> Self {
        Self { words: self.words }
    }

    pub fn to_small(&self) -> u128 {
        (self.words[0] as u128) + (self.words[1] as u128) * WORD_BASE
    }

    pub fn zero() -> Self {
        let words = [0u64; CREDIT_WORDS];
        Self { words }
    }

    pub fn from_small(value: u128) -> Self {
        let mut words = [0u64; CREDIT_WORDS];
        words[0] = (value % WORD_BASE) as u64;
        words[1] = (value / WORD_BASE) as u64;
        Self { words }
    }

    pub fn from_shifted(value: u128, shift: u16) -> Self {
        let offset = (shift / 64) as usize;
        let bits = (shift % 64) as u64;
        let factor = 1u64 << bits;
        let low = (value % WORD_BASE) as u64;
        let high = (value / WORD_BASE) as u64;
        let first = credit_words::multiply_word(low, factor, 0);
        let second = credit_words::multiply_word(high, factor, first.1);
        let mut words = [0u64; CREDIT_WORDS];
        words[offset] = first.0;
        words[offset + 1] = second.0;
        words[offset + 2] = second.1;
        Self { words }
    }

    pub fn compare(&self, other: &Self) -> CreditOrdering {
        let mut remaining = CREDIT_WORDS;
        while remaining > 0 {
            remaining -= 1;
            if self.words[remaining] < other.words[remaining] {
                return CreditOrdering::Less;
            }
            if self.words[remaining] > other.words[remaining] {
                return CreditOrdering::Greater;
            }
        }
        CreditOrdering::Equal
    }

    pub fn clamp(&mut self, bound: &Self) {
        if self.compare(bound) == CreditOrdering::Greater {
            *self = bound.copy();
        }
    }

    pub fn add_capped(&mut self, other: &Self, bound: &Self) {
        if self.add_assign(other) != 0 || self.compare(bound) == CreditOrdering::Greater {
            *self = bound.copy();
        }
    }

    pub fn rescale_capped(&mut self, source_scale: u64, target_scale: u64, bound: &Self) {
        if source_scale == target_scale {
            self.clamp(bound);
            return;
        }
        let remainder = self.divide_assign(source_scale);
        let fractional_product = (remainder as u128) * (target_scale as u128);
        let fraction = fractional_product / (source_scale as u128);
        let carry = self.multiply_assign(target_scale);
        if carry != 0 {
            *self = bound.copy();
        } else {
            let fractional = Self::from_small(fraction);
            self.add_capped(&fractional, bound);
        }
    }

    pub fn add_assign(&mut self, other: &Self) -> u64 {
        let mut carry = 0u64;
        let mut index = 0usize;
        while index < CREDIT_WORDS {
            let pair = add_word(self.words[index], other.words[index], carry);
            self.words[index] = pair.0;
            carry = pair.1;
            index += 1;
        }
        carry
    }

    pub fn multiply_assign(&mut self, factor: u64) -> u64 {
        let mut carry = 0u64;
        let mut index = 0usize;
        while index < CREDIT_WORDS {
            let pair = credit_words::multiply_word(self.words[index], factor, carry);
            self.words[index] = pair.0;
            carry = pair.1;
            index += 1;
        }
        carry
    }

    pub fn subtract_assign(&mut self, other: &Self) {
        let mut borrow = 0u64;
        let mut index = 0usize;
        while index < CREDIT_WORDS {
            let pair = credit_words::subtract_word(self.words[index], other.words[index], borrow);
            self.words[index] = pair.0;
            borrow = pair.1;
            index += 1;
        }
    }

    pub fn divide_assign(&mut self, divisor: u64) -> u64 {
        let mut remainder = 0u64;
        let mut remaining = CREDIT_WORDS;
        while remaining > 0 {
            remaining -= 1;
            let pair = credit_words::divide_word(remainder, self.words[remaining], divisor);
            self.words[remaining] = pair.0;
            remainder = pair.1;
        }
        remainder
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rescaling_keeps_the_fraction_of_a_period() {
        let bound = CreditBalance::from_small(1_000);
        let mut credits = CreditBalance::from_small(7);
        credits.rescale_capped(2, 3, &bound);
        assert_eq!(credits.to_small(), 10, "floor(7 * 3 / 2)");
        let mut credits = CreditBalance::from_small(7);
        credits.rescale_capped(2, 1_000, &bound);
        assert_eq!(credits.to_small(), 1_000, "capped");
    }
}
