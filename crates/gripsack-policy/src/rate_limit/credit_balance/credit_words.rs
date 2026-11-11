//! Exact carry, borrow, product and quotient steps over 64-bit credit words.

pub const WORD_BASE: u128 = 18446744073709551616;

pub fn add_word(left: u64, right: u64, carry: u64) -> (u64, u64) {
    let total = left as u128 + right as u128 + carry as u128;
    ((total % WORD_BASE) as u64, (total / WORD_BASE) as u64)
}

pub fn subtract_word(left: u64, right: u64, borrow: u64) -> (u64, u64) {
    let needed = right as u128 + borrow as u128;
    if left as u128 >= needed {
        ((left as u128 - needed) as u64, 0)
    } else {
        ((left as u128 + WORD_BASE - needed) as u64, 1)
    }
}

pub fn multiply_word(left: u64, scalar: u64, carry: u64) -> (u64, u64) {
    let total = left as u128 * scalar as u128 + carry as u128;
    ((total % WORD_BASE) as u64, (total / WORD_BASE) as u64)
}

pub fn divide_word(high: u64, low: u64, divisor: u64) -> (u64, u64) {
    let combined = high as u128 * WORD_BASE + low as u128;
    (
        (combined / divisor as u128) as u64,
        (combined % divisor as u128) as u64,
    )
}
