//! IEEE-754 rate admission without narrowing fractional or large declarations.
use vstd::prelude::*;

verus! {

pub const TOKEN_SCALE_BITS: u64 = 52;
pub const IMPLICIT_SIGNIFICAND_BIT: u64 = 4503599627370496;
#[cfg(verus_keep_ghost)]
pub const SIGNIFICAND_LIMIT: u64 = 9007199254740992;
pub const FRACTION_MASK: u64 = 4503599627370495;
pub const EXPONENT_MASK: u64 = 2047;
pub const EXPONENT_BIAS: u64 = 1023;
pub const MAX_FINITE_EXPONENT: u64 = 2046;

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum RatePeriod { Second, Minute, Hour }

impl RatePeriod {
    pub open spec fn nanos(self) -> u64 {
        match self {
            Self::Second => 1000000000u64,
            Self::Minute => 60000000000u64,
            Self::Hour => 3600000000000u64,
        }
    }

    pub fn nanoseconds(self) -> (duration: u64)
        ensures
            duration == self.nanos(),
            1000000000 <= duration <= 3600000000000,
    {
        match self {
            Self::Second => 1000000000,
            Self::Minute => 60000000000,
            Self::Hour => 3600000000000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub struct BinaryTokenRate {
    significand: u64,
    shift: u16,
    period: RatePeriod,
}

impl BinaryTokenRate {
    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        IMPLICIT_SIGNIFICAND_BIT <= self.significand < SIGNIFICAND_LIMIT
            && self.shift <= 1023
    }

    pub closed spec fn significand(self) -> u64 { self.significand }
    pub closed spec fn shift(self) -> u16 { self.shift }
    pub closed spec fn period(self) -> RatePeriod { self.period }

    pub open spec fn admitted(self) -> bool {
        IMPLICIT_SIGNIFICAND_BIT <= self.significand() < SIGNIFICAND_LIMIT
            && self.shift() <= 1023
    }

    pub fn components(self) -> (parts: (u64, u16, RatePeriod))
        ensures
            parts == (self.significand(), self.shift(), self.period()),
            IMPLICIT_SIGNIFICAND_BIT <= parts.0 < SIGNIFICAND_LIMIT,
            parts.1 <= 1023,
    {
        proof { use_type_invariant(&self); }
        (self.significand, self.shift, self.period)
    }
}

pub open spec fn admitted_rate_bits(bits: u64) -> bool {
    bits >> 63 == 0
        && EXPONENT_BIAS <= ((bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK) <= MAX_FINITE_EXPONENT
}

// f64 parsing and IEEE-754 to_bits are the observation boundary. Unlike an
// integer parser, this constructor preserves fractional/scientific rates and
// the full positive finite f64 range >= 1, including rates beyond u64::MAX.
pub fn admit_binary_rate(bits: u64, period: RatePeriod) -> (rate: Option<BinaryTokenRate>)
    ensures
        rate.is_some() <==> admitted_rate_bits(bits),
        rate.is_some() ==> rate.unwrap().period() == period,
        rate.is_some() ==> rate.unwrap().significand() == (bits & FRACTION_MASK) | IMPLICIT_SIGNIFICAND_BIT,
        rate.is_some() ==> rate.unwrap().shift() as u64 == ((bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK) - EXPONENT_BIAS,
{
    let exponent = (bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK;
    if bits >> 63 != 0 || !(EXPONENT_BIAS..=MAX_FINITE_EXPONENT).contains(&exponent) {
        return None;
    }
    let significand = (bits & FRACTION_MASK) | IMPLICIT_SIGNIFICAND_BIT;
    assert(IMPLICIT_SIGNIFICAND_BIT <= significand < SIGNIFICAND_LIMIT) by (bit_vector)
        requires significand == (bits & FRACTION_MASK) | IMPLICIT_SIGNIFICAND_BIT;
    Some(BinaryTokenRate { significand, shift: (exponent - EXPONENT_BIAS) as u16, period })
}

}
