//! Explicit migration of legacy floating token balances into exact credits.
#[cfg(verus_keep_ghost)]
use super::binary_rate::SIGNIFICAND_LIMIT;
use super::binary_rate::{
    EXPONENT_BIAS, EXPONENT_MASK, FRACTION_MASK, IMPLICIT_SIGNIFICAND_BIT, MAX_FINITE_EXPONENT,
    RatePeriod, TOKEN_SCALE_BITS,
};
use super::credit_balance::CreditBalance;
#[cfg(verus_keep_ghost)]
use vstd::arithmetic::power2::{
    lemma_pow2_adds, lemma_pow2_pos, lemma_pow2_strictly_increases, lemma2_to64, pow2,
};
#[cfg(verus_keep_ghost)]
use vstd::bits::lemma_u128_shr_is_div;
use vstd::prelude::*;

verus! {

pub open spec fn exponent(bits: u64) -> u64 { (bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK }

// Credits are floor(max(tokens, 0) * period_ns * 2^52). The floor can
// discard less than one credit, never create a token during migration.
pub open spec fn legacy_amount(bits: u64, period: RatePeriod) -> nat {
    let exp = exponent(bits);
    let fraction = bits & FRACTION_MASK;
    let significand = if exp == 0 { fraction } else { fraction | IMPLICIT_SIGNIFICAND_BIT };
    let numerator = (significand as nat) * (period.nanos() as nat);
    if bits >> 63 != 0 { 0 }
    else if exp >= EXPONENT_BIAS { numerator * pow2((exp - EXPONENT_BIAS) as nat) }
    else { numerator / pow2(if exp == 0 { 1022 } else { (EXPONENT_BIAS - exp) as nat }) }
}

pub fn admit_legacy_credits(bits: u64, period: RatePeriod) -> (credits: Option<CreditBalance>)
    ensures
        credits.is_some() <==> exponent(bits) <= MAX_FINITE_EXPONENT,
        credits.is_some() ==> credits.unwrap().view() == legacy_amount(bits, period),
{
    let exp = (bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK;
    if exp > MAX_FINITE_EXPONENT { return None; }
    if bits >> 63 != 0 { return Some(CreditBalance::zero()); }
    let fraction = bits & FRACTION_MASK;
    let significand = if exp == 0 { fraction } else { fraction | IMPLICIT_SIGNIFICAND_BIT };
    assert(significand < SIGNIFICAND_LIMIT) by (bit_vector)
        requires
            fraction == bits & FRACTION_MASK,
            significand == if exp == 0 { fraction } else { fraction | IMPLICIT_SIGNIFICAND_BIT };
    let nanos = period.nanoseconds();
    assert((significand as int) * (nanos as int) <= u128::MAX as int) by (nonlinear_arith);
    let numerator = (significand as u128) * (nanos as u128);
    if exp >= EXPONENT_BIAS {
        Some(CreditBalance::from_shifted(numerator, (exp - EXPONENT_BIAS) as u16))
    } else {
        let shift = if exp == 0 { 1022u64 } else { EXPONENT_BIAS - exp };
        if shift >= 128 {
            proof {
                lemma2_to64();
                lemma_pow2_adds(64, 64);
                if shift > 128 { lemma_pow2_strictly_increases(128, shift as nat); }
                assert((numerator as nat) < pow2(shift as nat));
                assert((numerator as nat) / pow2(shift as nat) == 0) by (nonlinear_arith)
                    requires (numerator as nat) < pow2(shift as nat), pow2(shift as nat) > 0;
            }
            Some(CreditBalance::zero())
        } else {
            proof { lemma_u128_shr_is_div(numerator, shift as u128); lemma_pow2_pos(shift as nat); }
            Some(CreditBalance::from_small(numerator >> shift))
        }
    }
}

}
