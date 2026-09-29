//! Exact carry, borrow, product and quotient steps over 64-bit credit words.
use vstd::prelude::*;

verus! {

pub const WORD_BASE: u128 = 18446744073709551616;

pub fn add_word(left: u64, right: u64, carry: u64) -> (result: (u64, u64))
    requires carry <= 1,
    ensures
        left as int + right as int + carry as int
            == result.0 as int + result.1 as int * WORD_BASE as int,
        result.1 <= 1,
    no_unwind
{
    let total = left as u128 + right as u128 + carry as u128;
    ((total % WORD_BASE) as u64, (total / WORD_BASE) as u64)
}

pub fn subtract_word(left: u64, right: u64, borrow: u64) -> (result: (u64, u64))
    requires borrow <= 1,
    ensures
        left as int - right as int - borrow as int
            == result.0 as int - result.1 as int * WORD_BASE as int,
        result.1 <= 1,
    no_unwind
{
    let needed = right as u128 + borrow as u128;
    if left as u128 >= needed {
        ((left as u128 - needed) as u64, 0)
    } else {
        ((left as u128 + WORD_BASE - needed) as u64, 1)
    }
}

pub fn multiply_word(left: u64, scalar: u64, carry: u64) -> (result: (u64, u64))
    requires scalar > 0, carry < scalar,
    ensures
        left as int * scalar as int + carry as int
            == result.0 as int + result.1 as int * WORD_BASE as int,
        result.1 < scalar,
    no_unwind
{
    assert((left as int) * (scalar as int) + (carry as int) <= u128::MAX as int)
        by (nonlinear_arith);
    let total = left as u128 * scalar as u128 + carry as u128;
    assert((total as int) < (scalar as int) * (WORD_BASE as int))
        by (nonlinear_arith)
        requires
            (total as int) == (left as int) * (scalar as int) + (carry as int),
            (left as int) < (WORD_BASE as int),
            carry < scalar,
            scalar > 0;
    assert(total / WORD_BASE < scalar as u128);
    ((total % WORD_BASE) as u64, (total / WORD_BASE) as u64)
}

pub fn divide_word(high: u64, low: u64, divisor: u64) -> (result: (u64, u64))
    requires divisor > 0, high < divisor,
    ensures
        high as int * WORD_BASE as int + low as int
            == result.0 as int * divisor as int + result.1 as int,
        result.1 < divisor,
    no_unwind
{
    let combined = high as u128 * WORD_BASE + low as u128;
    assert((combined as int) < (divisor as int) * (WORD_BASE as int))
        by (nonlinear_arith)
        requires
            (combined as int) == (high as int) * (WORD_BASE as int) + (low as int),
            high < divisor,
            (low as int) < (WORD_BASE as int);
    assert(combined / (divisor as u128) < WORD_BASE)
        by (nonlinear_arith)
        requires
            (combined as int) < (divisor as int) * (WORD_BASE as int),
            divisor > 0;
    assert((combined as int) == (combined as int) / (divisor as int) * (divisor as int)
        + (combined as int) % (divisor as int))
        by (nonlinear_arith)
        requires divisor > 0;
    ((combined / divisor as u128) as u64, (combined % divisor as u128) as u64)
}

}
