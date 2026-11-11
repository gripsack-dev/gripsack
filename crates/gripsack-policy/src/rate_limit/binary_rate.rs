//! IEEE-754 rate admission without narrowing fractional or large declarations.

pub const TOKEN_SCALE_BITS: u64 = 52;
pub const IMPLICIT_SIGNIFICAND_BIT: u64 = 4503599627370496;
pub const FRACTION_MASK: u64 = 4503599627370495;
pub const EXPONENT_MASK: u64 = 2047;
pub const EXPONENT_BIAS: u64 = 1023;
pub const MAX_FINITE_EXPONENT: u64 = 2046;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RatePeriod {
    Second,
    Minute,
    Hour,
}

impl RatePeriod {
    pub fn nanoseconds(self) -> u64 {
        match self {
            Self::Second => 1000000000,
            Self::Minute => 60000000000,
            Self::Hour => 3600000000000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BinaryTokenRate {
    significand: u64,
    shift: u16,
    period: RatePeriod,
}

impl BinaryTokenRate {
    pub fn components(self) -> (u64, u16, RatePeriod) {
        (self.significand, self.shift, self.period)
    }
}

// f64 parsing and IEEE-754 to_bits are the observation boundary. Unlike an
// integer parser, this constructor preserves fractional/scientific rates and
// the full positive finite f64 range >= 1, including rates beyond u64::MAX.
pub fn admit_binary_rate(bits: u64, period: RatePeriod) -> Option<BinaryTokenRate> {
    let exponent = (bits >> TOKEN_SCALE_BITS) & EXPONENT_MASK;
    if bits >> 63 != 0 || !(EXPONENT_BIAS..=MAX_FINITE_EXPONENT).contains(&exponent) {
        return None;
    }
    let significand = (bits & FRACTION_MASK) | IMPLICIT_SIGNIFICAND_BIT;
    Some(BinaryTokenRate {
        significand,
        shift: (exponent - EXPONENT_BIAS) as u16,
        period,
    })
}
