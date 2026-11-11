//! Explicit migration of legacy floating token balances into exact credits.
use super::binary_rate::{
    EXPONENT_BIAS, EXPONENT_MASK, FRACTION_MASK, IMPLICIT_SIGNIFICAND_BIT, MAX_FINITE_EXPONENT,
    RatePeriod, TOKEN_SCALE_BITS,
};
use super::credit_balance::CreditBalance;

// Credits are floor(max(tokens, 0) * period_ns * 2^52). The floor can
// discard less than one credit, never create a token during migration.

pub fn admit_legacy_credits(bits: u64, period: RatePeriod) -> Option<CreditBalance> {
    let exp = (bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK;
    if exp > MAX_FINITE_EXPONENT {
        return None;
    }
    if bits >> 63 != 0 {
        return Some(CreditBalance::zero());
    }
    let fraction = bits & FRACTION_MASK;
    let significand = if exp == 0 {
        fraction
    } else {
        fraction | IMPLICIT_SIGNIFICAND_BIT
    };
    let nanos = period.nanoseconds();
    let numerator = (significand as u128) * (nanos as u128);
    if exp >= EXPONENT_BIAS {
        Some(CreditBalance::from_shifted(
            numerator,
            (exp - EXPONENT_BIAS) as u16,
        ))
    } else {
        let shift = if exp == 0 {
            1022u64
        } else {
            EXPONENT_BIAS - exp
        };
        if shift >= 128 {
            Some(CreditBalance::zero())
        } else {
            Some(CreditBalance::from_small(numerator >> shift))
        }
    }
}
