//! Allocation-free credit arithmetic; eighteen words cover the admitted rate domain.
use vstd::prelude::*;
mod credit_words;
use credit_words::{WORD_BASE, add_word};
#[cfg(verus_keep_ghost)]
use vstd::arithmetic::power2::{lemma_pow2_adds, lemma2_to64, pow2};
#[cfg(verus_keep_ghost)]
use vstd::bits::{lemma_u64_pow2_no_overflow, lemma_u64_shl_is_mul};

verus! {

// Every accepted finite f64 rate >= 1 is m * 2^e / 2^52,
// with 2^52 <= m < 2^53 and 0 <= e <= 1023. Multiplication
// by the longest supported period (< 2^42 nanoseconds) needs
// fewer than 1118 bits. Eighteen 64-bit limbs leave headroom
// for adding two bounded balances without overflow.
pub const CREDIT_WORDS: usize = 18;

pub open spec fn place(index: nat) -> nat
    decreases index,
{
    if index == 0 { 1 } else { (WORD_BASE as nat) * place((index - 1) as nat) }
}

pub proof fn place_is_power(index: nat)
    ensures place(index) == pow2(64 * index),
    decreases index,
{
    lemma2_to64();
    if index > 0 {
        place_is_power((index - 1) as nat);
        lemma_pow2_adds(64, 64 * ((index - 1) as nat));
        assert(64 + 64 * ((index - 1) as nat) == 64 * index);
    }
}

pub open spec fn prefix_value(words: Seq<u64>, length: nat) -> nat
    recommends length <= words.len(),
    decreases length,
{
    if length == 0 { 0 }
    else {
        prefix_value(words, (length - 1) as nat)
            + (words[length as int - 1] as nat) * place((length - 1) as nat)
    }
}

proof fn positive_place(index: nat)
    ensures place(index) > 0,
    decreases index,
{
    if index > 0 {
        positive_place((index - 1) as nat);
    }
}

proof fn prefix_bound(words: Seq<u64>, length: nat)
    requires length <= words.len(),
    ensures prefix_value(words, length) < place(length),
    decreases length,
{
    if length > 0 {
        let previous = (length - 1) as nat;
        prefix_bound(words, previous);
        positive_place(previous);
        assert(prefix_value(words, previous) + (words[length as int - 1] as nat) * place(previous)
            < (WORD_BASE as nat) * place(previous))
            by (nonlinear_arith)
            requires
                prefix_value(words, previous) < place(previous),
                (words[length as int - 1] as nat) < WORD_BASE as nat,
                place(previous) > 0;
    }
}

proof fn zero_prefix(words: Seq<u64>, length: nat)
    requires
        length <= words.len(),
        forall|position: int| 0 <= position < length ==> #[trigger] words[position] == 0,
    ensures prefix_value(words, length) == 0,
    decreases length,
{
    if length > 0 {
        zero_prefix(words, (length - 1) as nat);
    }
}

proof fn zero_suffix_value(words: Seq<u64>, start: nat, end: nat)
    requires
        start <= end <= words.len(),
        forall|position: int| start <= position < end ==> #[trigger] words[position] == 0,
    ensures prefix_value(words, end) == prefix_value(words, start),
    decreases end - start,
{
    if end > start {
        zero_suffix_value(words, start, (end - 1) as nat);
    }
}

proof fn three_word_encoding(words: Seq<u64>, offset: nat)
    requires
        offset + 3 <= words.len(),
        forall|position: int| 0 <= position < offset ==> #[trigger] words[position] == 0,
        forall|position: int| offset + 3 <= position < words.len() ==> #[trigger] words[position] == 0,
    ensures
        prefix_value(words, words.len()) ==
            ((words[offset as int] as nat)
                + (words[offset as int + 1] as nat) * (WORD_BASE as nat)
                + (words[offset as int + 2] as nat) * (WORD_BASE as nat) * (WORD_BASE as nat))
            * place(offset),
{
    zero_prefix(words, offset);
    zero_suffix_value(words, offset + 3, words.len());
    assert(prefix_value(words, offset + 3) ==
        ((words[offset as int] as nat)
            + (words[offset as int + 1] as nat) * (WORD_BASE as nat)
            + (words[offset as int + 2] as nat) * (WORD_BASE as nat) * (WORD_BASE as nat))
        * place(offset))
        by (nonlinear_arith)
        requires
            prefix_value(words, offset) == 0,
            prefix_value(words, offset + 1)
                == prefix_value(words, offset) + (words[offset as int] as nat) * place(offset),
            prefix_value(words, offset + 2)
                == prefix_value(words, offset + 1) + (words[offset as int + 1] as nat) * place(offset + 1),
            prefix_value(words, offset + 3)
                == prefix_value(words, offset + 2) + (words[offset as int + 2] as nat) * place(offset + 2),
            place(offset + 1) == (WORD_BASE as nat) * place(offset),
            place(offset + 2) == (WORD_BASE as nat) * place(offset + 1);
}

proof fn equal_prefixes(left: Seq<u64>, right: Seq<u64>, length: nat)
    requires
        length <= left.len(), length <= right.len(),
        forall|position: int| 0 <= position < length ==> #[trigger] left[position] == right[position],
    ensures prefix_value(left, length) == prefix_value(right, length),
    decreases length,
{
    if length > 0 {
        equal_prefixes(left, right, (length - 1) as nat);
    }
}

proof fn unequal_word_orders_prefix(left: Seq<u64>, right: Seq<u64>, position: nat, length: nat)
    requires
        position < length <= left.len(), length <= right.len(),
        left[position as int] < right[position as int],
        forall|higher: int| position < higher < length ==> #[trigger] left[higher] == right[higher],
    ensures prefix_value(left, length) < prefix_value(right, length),
    decreases length - position,
{
    if length > position + 1 {
        unequal_word_orders_prefix(left, right, position, (length - 1) as nat);
    } else {
        prefix_bound(left, position);
        positive_place(position);
        assert(prefix_value(left, position) + (left[position as int] as nat) * place(position)
            < prefix_value(right, position) + (right[position as int] as nat) * place(position))
            by (nonlinear_arith)
            requires
                prefix_value(left, position) < place(position),
                place(position) > 0,
                left[position as int] < right[position as int];
    }
}

pub open spec fn tail_value(words: Seq<u64>, start: nat) -> nat
    recommends start <= words.len(),
    decreases words.len() - start,
{
    if start >= words.len() { 0 }
    else { (words[start as int] as nat) + (WORD_BASE as nat) * tail_value(words, start + 1) }
}

proof fn tail_and_prefix(words: Seq<u64>, start: nat)
    requires start <= words.len(),
    ensures prefix_value(words, start) + tail_value(words, start) * place(start)
        == prefix_value(words, words.len()),
    decreases words.len() - start,
{
    if start < words.len() {
        tail_and_prefix(words, start + 1);
        assert(prefix_value(words, start) + tail_value(words, start) * place(start)
            == prefix_value(words, words.len()))
            by (nonlinear_arith)
            requires
                prefix_value(words, start + 1) + tail_value(words, start + 1) * place(start + 1)
                    == prefix_value(words, words.len()),
                prefix_value(words, start + 1)
                    == prefix_value(words, start) + (words[start as int] as nat) * place(start),
                tail_value(words, start)
                    == (words[start as int] as nat) + (WORD_BASE as nat) * tail_value(words, start + 1),
                place(start + 1) == (WORD_BASE as nat) * place(start);
    } else {
        assert(tail_value(words, start) == 0);
    }
}

proof fn equal_tails(left: Seq<u64>, right: Seq<u64>, start: nat)
    requires
        left.len() == right.len(), start <= left.len(),
        forall|position: int| start <= position < left.len() ==> #[trigger] left[position] == right[position],
    ensures tail_value(left, start) == tail_value(right, start),
    decreases left.len() - start,
{
    if start < left.len() {
        equal_tails(left, right, start + 1);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, StructuralEq)]
pub enum CreditOrdering { Less, Equal, Greater }

pub struct CreditBalance {
    words: [u64; CREDIT_WORDS],
}

impl CreditBalance {
    pub closed spec fn view(&self) -> nat { prefix_value(self.words@, CREDIT_WORDS as nat) }

    pub proof fn bounded(&self)
        ensures self.view() < place(CREDIT_WORDS as nat),
    {
        prefix_bound(self.words@, CREDIT_WORDS as nat);
    }

    pub fn words(&self) -> (words: &[u64; CREDIT_WORDS])
        ensures prefix_value(words@, CREDIT_WORDS as nat) == self.view(),
    {
        &self.words
    }

    pub fn from_words(words: [u64; CREDIT_WORDS]) -> (balance: Self)
        ensures balance.view() == prefix_value(words@, CREDIT_WORDS as nat),
    {
        Self { words }
    }

    pub fn copy(&self) -> (balance: Self)
        ensures balance.view() == self.view(),
        no_unwind
    {
        Self { words: self.words }
    }

    pub fn to_small(&self) -> (value: u128)
        requires self.view() <= u128::MAX,
        ensures value == self.view(),
    {
        proof {
            tail_and_prefix(self.words@, 2);
            prefix_bound(self.words@, 2);
            reveal_with_fuel(place, 3);
            reveal_with_fuel(prefix_value, 3);
            assert(place(2) == (WORD_BASE as nat) * (WORD_BASE as nat));
            assert(tail_value(self.words@, 2) == 0) by (nonlinear_arith)
                requires
                    prefix_value(self.words@, 2) + tail_value(self.words@, 2) * place(2) == self.view(),
                    self.view() <= u128::MAX,
                    place(2) == (WORD_BASE as nat) * (WORD_BASE as nat);
            assert(self.view() == (self.words@[0] as nat)
                + (self.words@[1] as nat) * (WORD_BASE as nat))
                by (nonlinear_arith)
                requires
                    prefix_value(self.words@, 0) == 0,
                    prefix_value(self.words@, 1) == (self.words@[0] as nat) * place(0),
                    prefix_value(self.words@, 2) == prefix_value(self.words@, 1)
                        + (self.words@[1] as nat) * place(1),
                    place(0) == 1, place(1) == WORD_BASE as nat,
                    tail_value(self.words@, 2) == 0,
                    prefix_value(self.words@, 2) + tail_value(self.words@, 2) * place(2) == self.view();
        }
        (self.words[0] as u128) + (self.words[1] as u128) * WORD_BASE
    }

    pub fn zero() -> (balance: Self)
        ensures balance.view() == 0,
    {
        let words = [0u64; CREDIT_WORDS];
        proof { zero_prefix(words@, CREDIT_WORDS as nat); }
        Self { words }
    }

    pub fn from_small(value: u128) -> (balance: Self)
        ensures balance.view() == value,
    {
        let mut words = [0u64; CREDIT_WORDS];
        words[0] = (value % WORD_BASE) as u64;
        words[1] = (value / WORD_BASE) as u64;
        proof {
            zero_suffix_value(words@, 2, CREDIT_WORDS as nat);
            reveal_with_fuel(prefix_value, 3);
            reveal_with_fuel(place, 3);
            assert((value as int) == (words@[0] as int) + (words@[1] as int) * (WORD_BASE as int));
            assert(place(0) == 1);
            assert(place(1) == WORD_BASE as nat);
            assert(prefix_value(words@, 0) == 0);
            assert(prefix_value(words@, 1) == prefix_value(words@, 0) + (words@[0] as nat) * place(0));
            assert(prefix_value(words@, 1) == words@[0] as nat);
            assert(prefix_value(words@, 2) == (words@[0] as nat) + (words@[1] as nat) * (WORD_BASE as nat));
            assert(prefix_value(words@, 2) == value as nat);
        }
        Self { words }
    }

    pub fn from_shifted(value: u128, shift: u16) -> (balance: Self)
        requires shift <= 1023,
        ensures balance.view() == (value as nat) * pow2(shift as nat),
    {
        let offset = (shift / 64) as usize;
        let bits = (shift % 64) as u64;
        proof {
            lemma_u64_pow2_no_overflow(bits as nat);
            lemma_u64_shl_is_mul(1, bits);
        }
        let factor = 1u64 << bits;
        let low = (value % WORD_BASE) as u64;
        let high = (value / WORD_BASE) as u64;
        let first = credit_words::multiply_word(low, factor, 0);
        let second = credit_words::multiply_word(high, factor, first.1);
        let mut words = [0u64; CREDIT_WORDS];
        words[offset] = first.0;
        words[offset + 1] = second.0;
        words[offset + 2] = second.1;
        proof {
            three_word_encoding(words@, offset as nat);
            place_is_power(offset as nat);
            lemma_pow2_adds(64 * (offset as nat), bits as nat);
            assert(64 * (offset as nat) + bits as nat == shift as nat);
            assert((first.0 as nat) + (second.0 as nat) * (WORD_BASE as nat)
                + (second.1 as nat) * (WORD_BASE as nat) * (WORD_BASE as nat)
                == (value as nat) * (factor as nat))
                by (nonlinear_arith)
                requires
                    (value as int) == (low as int) + (high as int) * (WORD_BASE as int),
                    (low as int) * (factor as int)
                        == (first.0 as int) + (first.1 as int) * (WORD_BASE as int),
                    (high as int) * (factor as int) + (first.1 as int)
                        == (second.0 as int) + (second.1 as int) * (WORD_BASE as int);
            assert(prefix_value(words@, CREDIT_WORDS as nat)
                == (value as nat) * pow2(shift as nat))
                by (nonlinear_arith)
                requires
                    prefix_value(words@, CREDIT_WORDS as nat) ==
                        ((first.0 as nat) + (second.0 as nat) * (WORD_BASE as nat)
                            + (second.1 as nat) * (WORD_BASE as nat) * (WORD_BASE as nat))
                        * place(offset as nat),
                    pow2(shift as nat) == place(offset as nat) * (factor as nat),
                    (first.0 as nat) + (second.0 as nat) * (WORD_BASE as nat)
                        + (second.1 as nat) * (WORD_BASE as nat) * (WORD_BASE as nat)
                        == (value as nat) * (factor as nat);
        }
        Self { words }
    }

    pub fn compare(&self, other: &Self) -> (order: CreditOrdering)
        ensures
            (order == CreditOrdering::Less) <==> self.view() < other.view(),
            (order == CreditOrdering::Equal) <==> self.view() == other.view(),
            (order == CreditOrdering::Greater) <==> self.view() > other.view(),
        no_unwind
    {
        let mut remaining = CREDIT_WORDS;
        while remaining > 0
            invariant
                remaining <= CREDIT_WORDS,
                forall|position: int| remaining <= position < CREDIT_WORDS
                    ==> #[trigger] self.words@[position] == other.words@[position],
            decreases remaining,
        {
            remaining -= 1;
            if self.words[remaining] < other.words[remaining] {
                proof { unequal_word_orders_prefix(self.words@, other.words@, remaining as nat, CREDIT_WORDS as nat); }
                return CreditOrdering::Less;
            }
            if self.words[remaining] > other.words[remaining] {
                proof { unequal_word_orders_prefix(other.words@, self.words@, remaining as nat, CREDIT_WORDS as nat); }
                return CreditOrdering::Greater;
            }
        }
        proof { equal_prefixes(self.words@, other.words@, CREDIT_WORDS as nat); }
        CreditOrdering::Equal
    }

    pub fn clamp(&mut self, bound: &Self)
        ensures
            final(self).view() <= bound.view(),
            final(self).view() == if old(self).view() > bound.view() { bound.view() } else { old(self).view() },
        no_unwind
    {
        if self.compare(bound) == CreditOrdering::Greater {
            *self = bound.copy();
        }
    }

    pub fn add_capped(&mut self, other: &Self, bound: &Self)
        ensures
            final(self).view() <= bound.view(),
            final(self).view() == if old(self).view() + other.view() >= bound.view() {
                bound.view()
            } else {
                old(self).view() + other.view()
            },
        no_unwind
    {
        let ghost before = self.view();
        let carry = self.add_assign(other);
        if carry != 0 {
            proof {
                bound.bounded();
                assert(before + other.view() >= bound.view()) by (nonlinear_arith)
                    requires
                        carry >= 1,
                        self.view() + (carry as nat) * place(CREDIT_WORDS as nat) == before + other.view(),
                        bound.view() < place(CREDIT_WORDS as nat);
            }
            *self = bound.copy();
        } else if self.compare(bound) == CreditOrdering::Greater {
            *self = bound.copy();
        }
    }

    pub fn rescale_capped(&mut self, source_scale: u64, target_scale: u64, bound: &Self)
        requires source_scale > 0, target_scale > 0,
        ensures
            final(self).view() <= bound.view(),
            final(self).view() == if old(self).view() * (target_scale as nat) / (source_scale as nat) >= bound.view() {
                bound.view()
            } else {
                old(self).view() * (target_scale as nat) / (source_scale as nat)
            },
    {
        if source_scale == target_scale {
            assert(self.view() * (target_scale as nat) / (source_scale as nat) == self.view())
                by (nonlinear_arith)
                requires source_scale == target_scale, source_scale > 0;
            self.clamp(bound);
            return;
        }
        let ghost original = self.view();
        let remainder = self.divide_assign(source_scale);
        let ghost quotient = self.view();
        assert((remainder as int) * (target_scale as int) <= u128::MAX as int) by (nonlinear_arith);
        let fractional_product = (remainder as u128) * (target_scale as u128);
        let fraction = fractional_product / (source_scale as u128);
        proof {
            assert(original * (target_scale as nat) / (source_scale as nat)
                == quotient * (target_scale as nat) + fraction as nat)
                by (nonlinear_arith)
                requires
                    original == quotient * (source_scale as nat) + remainder as nat,
                    fractional_product == (remainder as nat) * (target_scale as nat),
                    fraction == (fractional_product as nat) / (source_scale as nat),
                    source_scale > 0;
        }
        let carry = self.multiply_assign(target_scale);
        if carry != 0 {
            proof {
                bound.bounded();
                assert(original * (target_scale as nat) / (source_scale as nat) >= bound.view())
                    by (nonlinear_arith)
                    requires
                        original * (target_scale as nat) / (source_scale as nat)
                            == quotient * (target_scale as nat) + fraction as nat,
                        self.view() + (carry as nat) * place(CREDIT_WORDS as nat)
                            == quotient * (target_scale as nat),
                        carry >= 1,
                        bound.view() < place(CREDIT_WORDS as nat);
            }
            *self = bound.copy();
        } else {
            let fractional = Self::from_small(fraction);
            self.add_capped(&fractional, bound);
        }
    }

    pub fn add_assign(&mut self, other: &Self) -> (carry: u64)
        ensures
            carry <= 1,
            final(self).view() + (carry as nat) * place(CREDIT_WORDS as nat)
                == old(self).view() + other.view(),
        no_unwind
    {
        let ghost before = self.words@;
        let mut carry = 0u64;
        let mut index = 0usize;
        while index < CREDIT_WORDS
            invariant
                index <= CREDIT_WORDS,
                carry <= 1,
                before.len() == CREDIT_WORDS,
                forall|position: int| index <= position < CREDIT_WORDS
                    ==> #[trigger] self.words@[position] == before[position],
                prefix_value(self.words@, index as nat) + (carry as nat) * place(index as nat)
                    == prefix_value(before, index as nat) + prefix_value(other.words@, index as nat),
            decreases CREDIT_WORDS - index,
        {
            let ghost previous = self.words@;
            let ghost old_carry = carry;
            let pair = add_word(self.words[index], other.words[index], carry);
            self.words[index] = pair.0;
            carry = pair.1;
            proof {
                equal_prefixes(previous, self.words@, index as nat);
                assert((before[index as int] as int) + (other.words@[index as int] as int) + (old_carry as int)
                    == (pair.0 as int) + (carry as int) * (WORD_BASE as int));
                assert(prefix_value(self.words@, index as nat) + (pair.0 as nat) * place(index as nat)
                    + (carry as nat) * (WORD_BASE as nat) * place(index as nat)
                    == prefix_value(before, index as nat) + (before[index as int] as nat) * place(index as nat)
                        + prefix_value(other.words@, index as nat) + (other.words@[index as int] as nat) * place(index as nat))
                    by (nonlinear_arith)
                    requires
                        prefix_value(self.words@, index as nat) + (old_carry as nat) * place(index as nat)
                            == prefix_value(before, index as nat) + prefix_value(other.words@, index as nat),
                        (before[index as int] as int) + (other.words@[index as int] as int) + (old_carry as int)
                            == (pair.0 as int) + (carry as int) * (WORD_BASE as int);
                let next = (index + 1) as nat;
                assert(prefix_value(self.words@, next) + (carry as nat) * place(next)
                    == prefix_value(before, next) + prefix_value(other.words@, next))
                    by (nonlinear_arith)
                    requires
                        prefix_value(self.words@, index as nat) + (pair.0 as nat) * place(index as nat)
                            + (carry as nat) * (WORD_BASE as nat) * place(index as nat)
                            == prefix_value(before, index as nat) + (before[index as int] as nat) * place(index as nat)
                                + prefix_value(other.words@, index as nat) + (other.words@[index as int] as nat) * place(index as nat),
                        prefix_value(self.words@, next) == prefix_value(self.words@, index as nat)
                            + (pair.0 as nat) * place(index as nat),
                        prefix_value(before, next) == prefix_value(before, index as nat)
                            + (before[index as int] as nat) * place(index as nat),
                        prefix_value(other.words@, next) == prefix_value(other.words@, index as nat)
                            + (other.words@[index as int] as nat) * place(index as nat),
                        place(next) == (WORD_BASE as nat) * place(index as nat);
            }
            index += 1;
        }
        carry
    }

    pub fn multiply_assign(&mut self, factor: u64) -> (carry: u64)
        requires factor > 0,
        ensures
            carry < factor,
            final(self).view() + (carry as nat) * place(CREDIT_WORDS as nat)
                == old(self).view() * (factor as nat),
        no_unwind
    {
        let ghost before = self.words@;
        let mut carry = 0u64;
        let mut index = 0usize;
        proof {
            assert(prefix_value(before, 0) == 0);
            assert(prefix_value(self.words@, 0) == 0);
            assert(place(0) == 1);
        }
        while index < CREDIT_WORDS
            invariant
                index <= CREDIT_WORDS,
                factor > 0,
                carry < factor,
                before.len() == CREDIT_WORDS,
                forall|position: int| index <= position < CREDIT_WORDS
                    ==> #[trigger] self.words@[position] == before[position],
                prefix_value(self.words@, index as nat) + (carry as nat) * place(index as nat)
                    == prefix_value(before, index as nat) * (factor as nat),
            decreases CREDIT_WORDS - index,
        {
            let ghost previous = self.words@;
            let ghost old_carry = carry;
            let pair = credit_words::multiply_word(self.words[index], factor, carry);
            self.words[index] = pair.0;
            carry = pair.1;
            proof {
                equal_prefixes(previous, self.words@, index as nat);
                let next = (index + 1) as nat;
                assert(prefix_value(self.words@, next) + (carry as nat) * place(next)
                    == prefix_value(before, next) * (factor as nat))
                    by (nonlinear_arith)
                    requires
                        prefix_value(self.words@, index as nat) + (old_carry as nat) * place(index as nat)
                            == prefix_value(before, index as nat) * (factor as nat),
                        (before[index as int] as int) * (factor as int) + (old_carry as int)
                            == (pair.0 as int) + (carry as int) * (WORD_BASE as int),
                        prefix_value(self.words@, next) == prefix_value(self.words@, index as nat)
                            + (pair.0 as nat) * place(index as nat),
                        prefix_value(before, next) == prefix_value(before, index as nat)
                            + (before[index as int] as nat) * place(index as nat),
                        place(next) == (WORD_BASE as nat) * place(index as nat);
            }
            index += 1;
        }
        carry
    }

    pub fn subtract_assign(&mut self, other: &Self)
        requires old(self).view() >= other.view(),
        ensures final(self).view() == old(self).view() - other.view(),
        no_unwind
    {
        let ghost before = self.words@;
        let mut borrow = 0u64;
        let mut index = 0usize;
        while index < CREDIT_WORDS
            invariant
                index <= CREDIT_WORDS,
                borrow <= 1,
                before.len() == CREDIT_WORDS,
                prefix_value(before, CREDIT_WORDS as nat) >= other.view(),
                forall|position: int| index <= position < CREDIT_WORDS
                    ==> #[trigger] self.words@[position] == before[position],
                (prefix_value(self.words@, index as nat) as int) - (borrow as int) * (place(index as nat) as int)
                    == (prefix_value(before, index as nat) as int) - (prefix_value(other.words@, index as nat) as int),
            decreases CREDIT_WORDS - index,
        {
            let ghost previous = self.words@;
            let ghost old_borrow = borrow;
            let pair = credit_words::subtract_word(self.words[index], other.words[index], borrow);
            self.words[index] = pair.0;
            borrow = pair.1;
            proof {
                equal_prefixes(previous, self.words@, index as nat);
                let next = (index + 1) as nat;
                assert((prefix_value(self.words@, next) as int) - (borrow as int) * (place(next) as int)
                    == (prefix_value(before, next) as int) - (prefix_value(other.words@, next) as int))
                    by (nonlinear_arith)
                    requires
                        (prefix_value(self.words@, index as nat) as int) - (old_borrow as int) * (place(index as nat) as int)
                            == (prefix_value(before, index as nat) as int) - (prefix_value(other.words@, index as nat) as int),
                        (before[index as int] as int) - (other.words@[index as int] as int) - (old_borrow as int)
                            == (pair.0 as int) - (borrow as int) * (WORD_BASE as int),
                        prefix_value(self.words@, next) == prefix_value(self.words@, index as nat)
                            + (pair.0 as nat) * place(index as nat),
                        prefix_value(before, next) == prefix_value(before, index as nat)
                            + (before[index as int] as nat) * place(index as nat),
                        prefix_value(other.words@, next) == prefix_value(other.words@, index as nat)
                            + (other.words@[index as int] as nat) * place(index as nat),
                        place(next) == (WORD_BASE as nat) * place(index as nat);
            }
            index += 1;
        }
        proof {
            prefix_bound(self.words@, CREDIT_WORDS as nat);
            assert(borrow == 0);
        }
    }

    pub fn divide_assign(&mut self, divisor: u64) -> (remainder: u64)
        requires divisor > 0,
        ensures
            remainder < divisor,
            old(self).view() == final(self).view() * (divisor as nat) + remainder as nat,
        no_unwind
    {
        let ghost before = self.words@;
        let mut remainder = 0u64;
        let mut remaining = CREDIT_WORDS;
        proof {
            assert(tail_value(before, CREDIT_WORDS as nat) == 0);
            assert(tail_value(self.words@, CREDIT_WORDS as nat) == 0);
        }
        while remaining > 0
            invariant
                remaining <= CREDIT_WORDS,
                divisor > 0,
                remainder < divisor,
                before.len() == CREDIT_WORDS,
                forall|position: int| 0 <= position < remaining
                    ==> #[trigger] self.words@[position] == before[position],
                tail_value(before, remaining as nat)
                    == tail_value(self.words@, remaining as nat) * (divisor as nat) + remainder as nat,
            decreases remaining,
        {
            remaining -= 1;
            let ghost previous = self.words@;
            let ghost old_remainder = remainder;
            let pair = credit_words::divide_word(remainder, self.words[remaining], divisor);
            self.words[remaining] = pair.0;
            remainder = pair.1;
            proof {
                equal_tails(previous, self.words@, (remaining + 1) as nat);
                assert(tail_value(before, remaining as nat)
                    == tail_value(self.words@, remaining as nat) * (divisor as nat) + remainder as nat)
                    by (nonlinear_arith)
                    requires
                        tail_value(before, (remaining + 1) as nat)
                            == tail_value(self.words@, (remaining + 1) as nat) * (divisor as nat) + old_remainder as nat,
                        (old_remainder as int) * (WORD_BASE as int) + (before[remaining as int] as int)
                            == (pair.0 as int) * (divisor as int) + (remainder as int),
                        tail_value(before, remaining as nat) == (before[remaining as int] as nat)
                            + (WORD_BASE as nat) * tail_value(before, (remaining + 1) as nat),
                        tail_value(self.words@, remaining as nat) == (pair.0 as nat)
                            + (WORD_BASE as nat) * tail_value(self.words@, (remaining + 1) as nat);
            }
        }
        proof {
            tail_and_prefix(before, 0);
            tail_and_prefix(self.words@, 0);
            assert(prefix_value(before, 0) == 0);
            assert(prefix_value(self.words@, 0) == 0);
            assert(place(0) == 1);
            assert(tail_value(before, 0) == prefix_value(before, CREDIT_WORDS as nat))
                by (nonlinear_arith)
                requires
                    prefix_value(before, 0) == 0, place(0) == 1,
                    before.len() == CREDIT_WORDS,
                    prefix_value(before, 0) + tail_value(before, 0) * place(0)
                        == prefix_value(before, before.len());
            assert(tail_value(self.words@, 0) == self.view())
                by (nonlinear_arith)
                requires
                    prefix_value(self.words@, 0) == 0, place(0) == 1,
                    self.words@.len() == CREDIT_WORDS,
                    self.view() == prefix_value(self.words@, CREDIT_WORDS as nat),
                    prefix_value(self.words@, 0) + tail_value(self.words@, 0) * place(0)
                        == prefix_value(self.words@, self.words@.len());
            assert(tail_value(before, 0) == self.view() * (divisor as nat) + remainder as nat);
            assert(prefix_value(before, CREDIT_WORDS as nat) == old(self).view());
        }
        remainder
    }
}

}
