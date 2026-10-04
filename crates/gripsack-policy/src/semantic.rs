//! Semantic normalization kernels for workspace identity (plan/0052 §5.1):
//! the length-delimited framing every v6 identity encoder hashes, the
//! canonical order every unordered declaration collection is normalized
//! to before encoding, and the proved boundary between semantic content
//! (hashed) and provenance (spans, line maps, labels — never hashed).
//!
//! Production binding: the v6 identity encoder
//! (gripsack-ir/src/workspace_v6/identity) frames every field through
//! `FrameBuffer` and normalizes every unordered collection (environment
//! maps, unordered string sets, mutation locks) through
//! `canonical_entries`, so these theorems bind the shipped digests.
//! SHA-256 collision resistance stays a trusted adapter (A1-04 records
//! it as such); everything up to the hash input is proved here.

use vstd::prelude::*;
// plain-cargo shim builds see no use of the seq/set/multiset lemmas;
// the `broadcast use` below (erased outside verification) needs them
#[allow(unused_imports)]
use vstd::{multiset::*, relations::*, seq_lib::*, set_lib::*};

verus! {

broadcast use {group_seq_properties, group_set_properties};

/* ----------------------------------------------------------------
 * Length-delimited framing
 * -------------------------------------------------------------- */

/// The 8-byte little-endian length header every field carries; matches
/// `u64::to_le_bytes` at the production call sites byte for byte.
pub open spec fn le64(value: u64) -> Seq<u8> {
    Seq::new(8, |i: int| ((value >> ((8 * i) as u64)) & 0xffu64) as u8)
}

/// One framed field: header bytes, then payload.
pub open spec fn frame(field: Seq<u8>) -> Seq<u8> {
    le64(field.len() as u64) + field
}

/// The exact byte stream a sequence of fields encodes to.
pub open spec fn encode_frames(frames: Seq<Seq<u8>>) -> Seq<u8>
    decreases frames.len(),
{
    if frames.len() == 0 {
        Seq::empty()
    } else {
        frame(frames[0]) + encode_frames(frames.drop_first())
    }
}

/// Distinct lengths never share a header.
pub proof fn lemma_le64_injective(a: u64, b: u64)
    requires
        le64(a) == le64(b),
    ensures
        a == b,
{
    assert(a == (le64(a)[0] as u64) | ((le64(a)[1] as u64) << 8) | ((le64(a)[2] as u64) << 16)
        | ((le64(a)[3] as u64) << 24) | ((le64(a)[4] as u64) << 32) | ((le64(a)[5] as u64) << 40)
        | ((le64(a)[6] as u64) << 48) | ((le64(a)[7] as u64) << 56)) by (bit_vector);
    assert(b == (le64(b)[0] as u64) | ((le64(b)[1] as u64) << 8) | ((le64(b)[2] as u64) << 16)
        | ((le64(b)[3] as u64) << 24) | ((le64(b)[4] as u64) << 32) | ((le64(b)[5] as u64) << 40)
        | ((le64(b)[6] as u64) << 48) | ((le64(b)[7] as u64) << 56)) by (bit_vector);
}

/// Encoding injectivity (A1-04): two admitted field sequences that hash
/// to the same bytes are the same sequence. Every tag, count, argument,
/// environment entry and canonical set member is recovered exactly.
pub proof fn lemma_encode_frames_injective(a: Seq<Seq<u8>>, b: Seq<Seq<u8>>)
    requires
        forall|i: int| 0 <= i < a.len() ==> #[trigger] a[i].len() <= u64::MAX as int,
        forall|i: int| 0 <= i < b.len() ==> #[trigger] b[i].len() <= u64::MAX as int,
        encode_frames(a) == encode_frames(b),
    ensures
        a == b,
    decreases a.len(), b.len(),
{
    if a.len() == 0 {
        if b.len() > 0 {
            // a framed field is never empty: header alone is 8 bytes
            assert(encode_frames(b).len() >= 8);
        }
    } else if b.len() == 0 {
        assert(encode_frames(a).len() >= 8);
    } else {
        let ea = encode_frames(a);
        let eb = encode_frames(b);
        assert(ea =~= frame(a[0]) + encode_frames(a.drop_first()));
        assert(eb =~= frame(b[0]) + encode_frames(b.drop_first()));
        assert(ea.subrange(0, 8) =~= le64(a[0].len() as u64));
        assert(eb.subrange(0, 8) =~= le64(b[0].len() as u64));
        lemma_le64_injective(a[0].len() as u64, b[0].len() as u64);
        assert(ea.subrange(8, 8 + a[0].len() as int) =~= a[0]);
        assert(eb.subrange(8, 8 + b[0].len() as int) =~= b[0]);
        assert(encode_frames(a.drop_first()) =~= ea.subrange(8 + a[0].len() as int, ea.len() as int));
        assert(encode_frames(b.drop_first()) =~= eb.subrange(8 + b[0].len() as int, eb.len() as int));
        assert forall|i: int| 0 <= i < a.drop_first().len() implies #[trigger]
            a.drop_first()[i].len() <= u64::MAX as int by {
            assert(a[i + 1].len() <= u64::MAX as int);
        };
        assert forall|i: int| 0 <= i < b.drop_first().len() implies #[trigger]
            b.drop_first()[i].len() <= u64::MAX as int by {
            assert(b[i + 1].len() <= u64::MAX as int);
        };
        lemma_encode_frames_injective(a.drop_first(), b.drop_first());
        assert(a =~= seq![a[0]] + a.drop_first());
        assert(b =~= seq![b[0]] + b.drop_first());
    }
}

/// The 8-byte length header a field of `length` bytes carries — the
/// exact bytes the streaming production encoder (`Encoder::field`,
/// gripsack-ir/src/workspace_v6/identity/encode.rs) feeds the hash
/// before each payload. Zero allocation: the header is computed in
/// place and the caller streams it straight into SHA-256.
pub fn length_header(length: u64) -> (result: [u8; 8])
    ensures
        result@ == le64(length),
{
    let mut out = [0u8; 8];
    let mut shift = 0u64;
    while shift < 64
        invariant
            shift <= 64,
            shift % 8 == 0,
            forall|k: int| 0 <= k < (shift / 8) as int ==> out@[k] == le64(length)[k],
        decreases 64 - shift,
    {
        proof {
            assert(((length >> shift) & 0xffu64) <= 0xff) by (bit_vector);
        }
        let byte = ((length >> shift) & 0xffu64) as u8;
        out[(shift / 8) as usize] = byte;
        proof {
            let k = (shift / 8) as int;
            assert(8 * k == shift as int);
            assert(le64(length)[k] == byte);
        }
        shift += 8;
    }
    proof {
        assert_seqs_equal!(out@ == le64(length));
    }
    out
}

/* ----------------------------------------------------------------
 * Lexicographic byte order (matches Rust's `str`/`[u8]` ordering)
 * -------------------------------------------------------------- */

/// Three-way comparison: negative, zero or positive as a is below,
/// equal to or above b in byte-lexicographic order.
pub open spec fn compare_bytes(a: Seq<u8>, b: Seq<u8>) -> int
    decreases a.len(), b.len(),
{
    if a.len() == 0 && b.len() == 0 {
        0
    } else if a.len() == 0 {
        -1
    } else if b.len() == 0 {
        1
    } else if a[0] < b[0] {
        -1
    } else if a[0] > b[0] {
        1
    } else {
        compare_bytes(a.drop_first(), b.drop_first())
    }
}

/// A field is below-or-equal another in canonical order.
pub open spec fn leq_bytes(a: Seq<u8>, b: Seq<u8>) -> bool {
    compare_bytes(a, b) <= 0
}

pub proof fn lemma_compare_bytes_self(a: Seq<u8>)
    ensures
        compare_bytes(a, a) == 0,
    decreases a.len(),
{
    if a.len() > 0 {
        lemma_compare_bytes_self(a.drop_first());
    }
}

pub proof fn lemma_compare_bytes_antisymmetric(a: Seq<u8>, b: Seq<u8>)
    ensures
        compare_bytes(a, b) == -compare_bytes(b, a),
    decreases a.len() + b.len(),
{
    if a.len() > 0 && b.len() > 0 && a[0] == b[0] {
        lemma_compare_bytes_antisymmetric(a.drop_first(), b.drop_first());
    }
}

pub proof fn lemma_compare_bytes_equal(a: Seq<u8>, b: Seq<u8>)
    requires
        compare_bytes(a, b) == 0,
    ensures
        a == b,
    decreases a.len() + b.len(),
{
    if a.len() > 0 && b.len() > 0 {
        lemma_compare_bytes_equal(a.drop_first(), b.drop_first());
        assert(a =~= seq![a[0]] + a.drop_first());
        assert(b =~= seq![b[0]] + b.drop_first());
    }
}

pub proof fn lemma_compare_bytes_transitive(a: Seq<u8>, b: Seq<u8>, c: Seq<u8>)
    requires
        compare_bytes(a, b) <= 0,
        compare_bytes(b, c) <= 0,
    ensures
        compare_bytes(a, c) <= 0,
    decreases a.len() + b.len() + c.len(),
{
    if a.len() == 0 {
    } else if b.len() == 0 {
        // compare_bytes(a, b) == 1: contradicts the requirement
    } else if c.len() == 0 {
        // compare_bytes(b, c) == 1: contradicts the requirement
    } else if a[0] < b[0] {
        // compare_bytes(b, c) <= 0 forces b[0] <= c[0]
        assert(b[0] <= c[0]);
        assert(a[0] < c[0]);
    } else if a[0] == b[0] {
        if b[0] < c[0] {
        } else if b[0] == c[0] {
            lemma_compare_bytes_transitive(a.drop_first(), b.drop_first(), c.drop_first());
        } else {
            // compare_bytes(b, c) == 1: contradicts the requirement
        }
    } else {
        // compare_bytes(a, b) == 1: contradicts the requirement
    }
}

/// Comparing past a shared prefix.
pub proof fn lemma_compare_skip_prefix(a: Seq<u8>, b: Seq<u8>, k: int)
    requires
        0 <= k <= a.len(),
        k <= b.len(),
        forall|i: int| 0 <= i < k ==> a[i] == b[i],
    ensures
        compare_bytes(a, b) == compare_bytes(
            a.subrange(k, a.len() as int),
            b.subrange(k, b.len() as int),
        ),
    decreases k,
{
    if k > 0 {
        assert(a[0] == b[0]);
        assert(compare_bytes(a, b) == compare_bytes(a.drop_first(), b.drop_first()));
        lemma_compare_skip_prefix(a.drop_first(), b.drop_first(), k - 1);
        assert(a.subrange(k, a.len() as int) =~= a.drop_first().subrange(
            k - 1,
            a.len() as int - 1,
        ));
        assert(b.subrange(k, b.len() as int) =~= b.drop_first().subrange(
            k - 1,
            b.len() as int - 1,
        ));
        assert(compare_bytes(a.subrange(k, a.len() as int), b.subrange(k, b.len() as int))
            == compare_bytes(a.drop_first(), b.drop_first()));
    } else {
        assert(a.subrange(0, a.len() as int) =~= a);
        assert(b.subrange(0, b.len() as int) =~= b);
    }
}

/// Byte-lexicographic order is total (used by the vstd sortedness
/// lemmas and by the canonical-form uniqueness theorem).
pub proof fn lemma_leq_bytes_total_ordering()
    ensures
        total_ordering(|x: Seq<u8>, y: Seq<u8>| leq_bytes(x, y)),
{
    assert(reflexive(|x: Seq<u8>, y: Seq<u8>| leq_bytes(x, y))) by {
        assert forall|x: Seq<u8>| #[trigger] leq_bytes(x, x) by {
            lemma_compare_bytes_self(x);
        };
    };
    assert(antisymmetric(|x: Seq<u8>, y: Seq<u8>| leq_bytes(x, y))) by {
        assert forall|x: Seq<u8>, y: Seq<u8>| #[trigger] leq_bytes(x, y) && #[trigger] leq_bytes(
            y,
            x,
        ) implies x == y by {
            lemma_compare_bytes_antisymmetric(x, y);
            lemma_compare_bytes_equal(x, y);
        };
    };
    assert(transitive(|x: Seq<u8>, y: Seq<u8>| leq_bytes(x, y))) by {
        assert forall|x: Seq<u8>, y: Seq<u8>, z: Seq<u8>| #[trigger] leq_bytes(x, y)
            && #[trigger] leq_bytes(y, z) implies leq_bytes(x, z) by {
            lemma_compare_bytes_transitive(x, y, z);
        };
    };
    assert(strongly_connected(|x: Seq<u8>, y: Seq<u8>| leq_bytes(x, y))) by {
        assert forall|x: Seq<u8>, y: Seq<u8>| #[trigger] leq_bytes(x, y) || #[trigger] leq_bytes(
            y,
            x,
        ) by {
            lemma_compare_bytes_antisymmetric(x, y);
        };
    };
}

/// Executable three-way byte comparison.
pub fn bytes_cmp(a: &[u8], b: &[u8]) -> (result: i8)
    ensures
        result as int == compare_bytes(a@, b@),
{
    let mut i: usize = 0;
    while i < a.len() && i < b.len()
        invariant
            i <= a.len(),
            i <= b.len(),
            forall|k: int| 0 <= k < i as int ==> a@[k] == b@[k],
        decreases a.len() - i,
    {
        if a[i] < b[i] {
            proof {
                lemma_compare_skip_prefix(a@, b@, i as int);
            }
            return -1;
        }
        if a[i] > b[i] {
            proof {
                lemma_compare_skip_prefix(a@, b@, i as int);
            }
            return 1;
        }
        i += 1;
    }
    proof {
        lemma_compare_skip_prefix(a@, b@, i as int);
    }
    if i == a.len() && i == b.len() {
        0
    } else if i == a.len() {
        -1
    } else {
        1
    }
}

/// Executable byte-order decision.
pub fn bytes_leq(a: &[u8], b: &[u8]) -> (result: bool)
    ensures
        result == leq_bytes(a@, b@),
{
    bytes_cmp(a, b) <= 0
}

/* ----------------------------------------------------------------
 * Canonical order for unordered declaration collections
 * -------------------------------------------------------------- */

/// The pair view of one entry: sort key bytes then secondary bytes
/// (for strings the secondary part is empty; for mutation locks it is
/// the lock key under its scope; for environment entries it is empty
/// because keys are unique at admission).
pub open spec fn entry_pair<V>(entry: (&[u8], &[u8], V)) -> (Seq<u8>, Seq<u8>) {
    (entry.0@, entry.1@)
}

/// The pair view of an entry sequence.
pub open spec fn entry_pairs<V>(entries: Seq<(&[u8], &[u8], V)>) -> Seq<(Seq<u8>, Seq<u8>)> {
    entries.map(|_i: int, entry: (&[u8], &[u8], V)| entry_pair(entry))
}

/// Canonical order on entries: lexicographic on the first component,
/// ties broken by the second — exactly `(&str, &str)` tuple ordering.
pub open spec fn leq_pair(x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>)) -> bool {
    let first = compare_bytes(x.0, y.0);
    first < 0 || (first == 0 && compare_bytes(x.1, y.1) <= 0)
}

/// The canonical order as a `spec_fn` value, so every sortedness fact
/// cites one identical trigger term.
pub open spec fn leq_pair_fn() -> spec_fn((Seq<u8>, Seq<u8>), (Seq<u8>, Seq<u8>)) -> bool {
    |x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>)| leq_pair(x, y)
}

pub proof fn lemma_leq_pair_reflexive(x: (Seq<u8>, Seq<u8>))
    ensures
        leq_pair(x, x),
{
    lemma_compare_bytes_self(x.0);
    lemma_compare_bytes_self(x.1);
}

pub proof fn lemma_leq_pair_antisymmetric(x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>))
    requires
        leq_pair(x, y),
        leq_pair(y, x),
    ensures
        x == y,
{
    lemma_compare_bytes_antisymmetric(x.0, y.0);
    lemma_compare_bytes_antisymmetric(x.1, y.1);
    if compare_bytes(x.0, y.0) == 0 {
        lemma_compare_bytes_equal(x.0, y.0);
        lemma_compare_bytes_equal(x.1, y.1);
    }
}

pub proof fn lemma_leq_pair_transitive(x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>), z: (
    Seq<u8>,
    Seq<u8>,
))
    requires
        leq_pair(x, y),
        leq_pair(y, z),
    ensures
        leq_pair(x, z),
{
    lemma_compare_bytes_antisymmetric(y.0, x.0);
    lemma_compare_bytes_antisymmetric(z.0, y.0);
    lemma_compare_bytes_transitive(x.0, y.0, z.0);
    if compare_bytes(x.0, z.0) == 0 {
        lemma_compare_bytes_equal(x.0, z.0);
        lemma_compare_bytes_equal(x.0, y.0);
        lemma_compare_bytes_equal(y.0, z.0);
        lemma_compare_bytes_transitive(x.1, y.1, z.1);
    }
}

pub proof fn lemma_leq_pair_connected(x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>))
    ensures
        leq_pair(x, y) || leq_pair(y, x),
{
    lemma_compare_bytes_antisymmetric(x.0, y.0);
    lemma_compare_bytes_antisymmetric(x.1, y.1);
}

/// The pair order is total.
pub proof fn lemma_leq_pair_total_ordering()
    ensures
        total_ordering(leq_pair_fn()),
{
    assert(reflexive(leq_pair_fn())) by {
        assert forall|x: (Seq<u8>, Seq<u8>)| #[trigger] leq_pair(x, x) by {
            lemma_leq_pair_reflexive(x);
        };
    };
    assert(antisymmetric(leq_pair_fn())) by {
        assert forall|x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>)| #[trigger] leq_pair(x, y)
            && #[trigger] leq_pair(y, x) implies x == y by {
            lemma_leq_pair_antisymmetric(x, y);
        };
    };
    assert(transitive(leq_pair_fn())) by {
        assert forall|x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>), z: (Seq<u8>, Seq<u8>)|
            #[trigger] leq_pair(x, y) && #[trigger] leq_pair(y, z) implies leq_pair(x, z) by {
            lemma_leq_pair_transitive(x, y, z);
        };
    };
    assert(strongly_connected(leq_pair_fn())) by {
        assert forall|x: (Seq<u8>, Seq<u8>), y: (Seq<u8>, Seq<u8>)| #[trigger] leq_pair(x, y)
            || #[trigger] leq_pair(y, x) by {
            lemma_leq_pair_connected(x, y);
        };
    };
}

fn entry_leq<V>(a: (&[u8], &[u8], V), b: (&[u8], &[u8], V)) -> (result: bool)
    ensures
        result == leq_pair(entry_pair(a), entry_pair(b)),
{
    let first = bytes_cmp(a.0, b.0);
    if first < 0 {
        true
    } else if first == 0 {
        bytes_leq(a.1, b.1)
    } else {
        false
    }
}

fn entry_equal<V>(a: (&[u8], &[u8], V), b: (&[u8], &[u8], V)) -> (result: bool)
    ensures
        result == (entry_pair(a) == entry_pair(b)),
{
    let first = bytes_cmp(a.0, b.0) == 0;
    let second = bytes_cmp(a.1, b.1) == 0;
    proof {
        if first {
            lemma_compare_bytes_equal(a.0@, b.0@);
        }
        if second {
            lemma_compare_bytes_equal(a.1@, b.1@);
        }
        if first && second {
        } else {
            lemma_compare_bytes_self(a.0@);
            lemma_compare_bytes_self(a.1@);
        }
    }
    first && second
}

/// Inserting an element ahead of the first position it precedes keeps a
/// sorted sequence sorted.
pub proof fn lemma_sorted_insert(s: Seq<(Seq<u8>, Seq<u8>)>, e: (Seq<u8>, Seq<u8>), j: int)
    requires
        sorted_by(s, leq_pair_fn()),
        0 <= j <= s.len(),
        forall|k: int| 0 <= k < j ==> !leq_pair(e, s[k]),
        j == s.len() || leq_pair(e, s[j]),
    ensures
        sorted_by(s.subrange(0, j).push(e) + s.subrange(j, s.len() as int), leq_pair_fn()),
{
    let new = s.subrange(0, j).push(e) + s.subrange(j, s.len() as int);
    assert(new.len() == s.len() + 1);
    assert forall|x: int, y: int| 0 <= x < y < new.len() implies #[trigger] leq_pair(new[x], new[y]) by {
        if y < j {
            assert(new[x] == s[x]);
            assert(new[y] == s[y]);
            assert(leq_pair_fn()(s[x], s[y]));
        } else if y == j {
            assert(new[x] == s[x]);
            assert(new[y] == e);
            lemma_leq_pair_connected(e, s[x]);
            assert(leq_pair(s[x], e));
        } else if x < j {
            assert(new[x] == s[x]);
            assert(new[y] == s[y - 1]);
            lemma_leq_pair_connected(e, s[x]);
            assert(leq_pair(s[x], e));
            assert(leq_pair(e, s[j]));
            if j < y - 1 {
                assert(leq_pair_fn()(s[j], s[y - 1]));
            } else {
                lemma_leq_pair_reflexive(s[j]);
            }
            lemma_leq_pair_transitive(s[x], e, s[j]);
            lemma_leq_pair_transitive(s[x], s[j], s[y - 1]);
        } else if x == j {
            assert(new[x] == e);
            assert(new[y] == s[y - 1]);
            assert(leq_pair(e, s[j]));
            if j < y - 1 {
                assert(leq_pair_fn()(s[j], s[y - 1]));
            } else {
                lemma_leq_pair_reflexive(s[j]);
            }
            lemma_leq_pair_transitive(e, s[j], s[y - 1]);
        } else {
            assert(new[x] == s[x - 1]);
            assert(new[y] == s[y - 1]);
            assert(leq_pair_fn()(s[x - 1], s[y - 1]));
        }
    };
    assert forall|x: int, y: int| 0 <= x < y < new.len() implies #[trigger] leq_pair_fn()(
        new[x],
        new[y],
    ) by {
    };
}

/// The insertion never introduces a duplicate pair.
pub proof fn lemma_nodup_insert(s: Seq<(Seq<u8>, Seq<u8>)>, e: (Seq<u8>, Seq<u8>), j: int)
    requires
        sorted_by(s, leq_pair_fn()),
        s.no_duplicates(),
        0 <= j <= s.len(),
        forall|k: int| 0 <= k < j ==> !leq_pair(e, s[k]),
        j == s.len() || (leq_pair(e, s[j]) && e != s[j]),
    ensures
        (s.subrange(0, j).push(e) + s.subrange(j, s.len() as int)).no_duplicates(),
{
    assert forall|k: int| 0 <= k < s.len() implies s[k] != e by {
        if k < j {
            lemma_leq_pair_reflexive(e);
        } else if k == j {
        } else {
            // e <= s[j] <= s[k]; if s[k] == e then e == s[j]
            assert(leq_pair_fn()(s[j], s[k]));
            if s[k] == e {
                lemma_leq_pair_antisymmetric(e, s[j]);
            }
        }
    };
    let prefix = s.subrange(0, j).push(e);
    let suffix = s.subrange(j, s.len() as int);
    assert(prefix.no_duplicates()) by {
        assert forall|x: int, y: int| 0 <= x < y < prefix.len() implies prefix[x] != prefix[y] by {
            if y < j {
                assert(prefix[x] == s[x]);
                assert(prefix[y] == s[y]);
            } else {
                assert(prefix[y] == e);
                assert(prefix[x] == s[x]);
            }
        };
    };
    assert(suffix.no_duplicates()) by {
        assert forall|x: int, y: int| 0 <= x < y < suffix.len() implies suffix[x] != suffix[y] by {
            assert(suffix[x] == s[j + x]);
            assert(suffix[y] == s[j + y]);
        };
    };
    lemma_no_dup_in_concat(prefix, suffix);
}

/// Canonicalize an unordered declaration collection: sort by (first,
/// second) byte order and drop duplicate pairs — the normalization the
/// production identity encoder applies to environment maps, unordered
/// string sets and mutation locks before framing them. The attached
/// values ride along untouched; callers whose keys are unique (the
/// admitted environment map) get back exactly their input order made
/// canonical.
///
/// The entry is one of the declared inputs (values ride along with
/// their keys through canonicalization).
pub open spec fn entry_from<V>(entry: (&[u8], &[u8], V), values: Seq<(&[u8], &[u8], V)>) -> bool {
    exists|j: int| 0 <= j < values.len() && #[trigger] values[j] == entry
}

pub fn canonical_entries<'a, V: Copy>(
    values: &[(&'a [u8], &'a [u8], V)],
) -> (result: Vec<(&'a [u8], &'a [u8], V)>)
    ensures
        result@.len() <= values@.len(),
        sorted_by(entry_pairs(result@), leq_pair_fn()),
        entry_pairs(result@).no_duplicates(),
        entry_pairs(result@).to_set() =~= entry_pairs(values@).to_set(),
        forall|k: int| 0 <= k < result@.len() ==> #[trigger] entry_from(result@[k], values@),
{
    let mut result: Vec<(&[u8], &[u8], V)> = Vec::new();
    let mut i: usize = 0;
    while i < values.len()
        invariant
            i <= values@.len(),
            result@.len() <= i,
            sorted_by(entry_pairs(result@), leq_pair_fn()),
            entry_pairs(result@).no_duplicates(),
            entry_pairs(result@).to_set() =~= entry_pairs(values@.subrange(0, i as int)).to_set(),
            forall|k: int| 0 <= k < result@.len() ==> #[trigger] entry_from(result@[k], values@),
        decreases values.len() - i,
    {
        let ghost previous_result = result@;
        let ghost consumed = values@.subrange(0, i as int);
        let entry = values[i];
        let mut j: usize = 0;
        let mut placed = false;
        while !placed && j < result.len()
            invariant
                j <= result@.len(),
                result@ == previous_result,
                placed ==> leq_pair(entry_pair(entry), entry_pair(result@[j as int])),
                forall|k: int| 0 <= k < j as int ==> !leq_pair(
                    entry_pair(entry),
                    entry_pair(result@[k]),
                ),
            decreases result.len() - j, if placed { 0int } else { 1int },
        {
            if entry_leq(entry, result[j]) {
                placed = true;
            } else {
                j += 1;
            }
        }
        proof {
            assert(entry_pairs(values@.subrange(0, i as int + 1)) =~= entry_pairs(consumed).push(
                entry_pair(entry),
            )) by {
                assert_seqs_equal!(
                    entry_pairs(values@.subrange(0, i as int + 1))
                        == entry_pairs(consumed).push(entry_pair(entry))
                );
            };
        }
        if j < result.len() && entry_equal(entry, result[j]) {
            // duplicate pair: the canonical form drops it
            proof {
                assert(entry_pairs(result@).to_set().contains(entry_pair(entry))) by {
                    assert(entry_pairs(result@)[j as int] == entry_pair(entry));
                };
                assert forall|v| #[trigger]
                    entry_pairs(values@.subrange(0, i as int + 1)).to_set().contains(v)
                    implies entry_pairs(consumed).to_set().contains(v) by {
                    if v == entry_pair(entry) {
                        assert(entry_pairs(consumed).to_set().contains(entry_pair(entry)));
                    } else {
                        broadcast use Seq::to_set_ensures, lemma_seq_contains;

                        let idx = choose|idx: int|
                            0 <= idx < i + 1 && #[trigger] entry_pairs(values@.subrange(0, i as int + 1))[idx]
                                == v;
                        assert(entry_pairs(consumed)[idx] == v);
                    }
                };
            }
        } else {
            proof {
                lemma_leq_pair_reflexive(entry_pair(entry));
                assert forall|k: int| 0 <= k < j as int implies !leq_pair(
                    entry_pair(entry),
                    entry_pairs(result@)[k],
                ) by {
                    assert(entry_pairs(result@)[k] == entry_pair(result@[k]));
                };
                if j < result.len() {
                    assert(placed);
                    assert(entry_pairs(result@)[j as int] == entry_pair(result@[j as int]));
                    assert(entry_pair(entry) != entry_pairs(result@)[j as int]);
                }
                lemma_sorted_insert(entry_pairs(result@), entry_pair(entry), j as int);
                lemma_nodup_insert(entry_pairs(result@), entry_pair(entry), j as int);
            }
            result.insert(j, entry);
            proof {
                let ghost pre_pairs = entry_pairs(previous_result);
                assert(entry_pairs(result@) =~= pre_pairs.subrange(0, j as int).push(
                    entry_pair(entry),
                ) + pre_pairs.subrange(j as int, pre_pairs.len() as int)) by {
                    assert_seqs_equal!(
                        entry_pairs(result@)
                            == pre_pairs.subrange(0, j as int).push(entry_pair(entry))
                                + pre_pairs.subrange(j as int, pre_pairs.len() as int)
                    );
                };
                assert forall|v| #[trigger] entry_pairs(result@).to_set().contains(v) <==> {
                    ||| entry_pairs(consumed).to_set().contains(v) ||| v == entry_pair(entry)
                } by {
                    broadcast use Seq::to_set_ensures, lemma_seq_contains;

                    if v == entry_pair(entry) {
                        assert(entry_pairs(result@)[j as int] == entry_pair(entry));
                    } else if entry_pairs(result@).to_set().contains(v) {
                        let idx = choose|idx: int|
                            0 <= idx < result@.len() && #[trigger] entry_pairs(result@)[idx] == v;
                        if idx < j as int {
                            assert(entry_pairs(previous_result)[idx] == v);
                        } else {
                            assert(entry_pairs(previous_result)[idx - 1] == v);
                        }
                    }
                };
                assert forall|v| #[trigger]
                    entry_pairs(values@.subrange(0, i as int + 1)).to_set().contains(v) <==> {
                    ||| entry_pairs(consumed).to_set().contains(v) ||| v == entry_pair(entry)
                } by {
                    broadcast use Seq::to_set_ensures, lemma_seq_contains;

                    if v == entry_pair(entry) {
                        assert(entry_pairs(values@.subrange(0, i as int + 1))[i as int]
                            == entry_pair(entry));
                    } else if entry_pairs(values@.subrange(0, i as int + 1)).to_set().contains(v) {
                        let idx = choose|idx: int|
                            0 <= idx < i + 1 && #[trigger] entry_pairs(values@.subrange(0, i as int + 1))[idx]
                                == v;
                        assert(entry_pairs(consumed)[idx] == v);
                    }
                };
                assert forall|k: int| 0 <= k < result@.len() implies #[trigger] entry_from(
                    result@[k],
                    values@,
                ) by {
                    if k < j as int {
                        assert(result@[k] == previous_result[k]);
                        assert(entry_from(previous_result[k], values@));
                    } else if k == j as int {
                        assert(result@[k] == values@[i as int]);
                    } else {
                        assert(result@[k] == previous_result[k - 1]);
                        assert(entry_from(previous_result[k - 1], values@));
                    }
                };
            }
        }
        i += 1;
    }
    proof {
        assert(values@.subrange(0, values@.len() as int) =~= values@);
    }
    result
}

/* ----------------------------------------------------------------
 * The semantic/provenance representation relation
 * -------------------------------------------------------------- */

/// Drop adjacent duplicates from an already-sorted sequence.
pub open spec fn dedup_sorted(values: Seq<(Seq<u8>, Seq<u8>)>) -> Seq<(Seq<u8>, Seq<u8>)>
    decreases values.len(),
{
    if values.len() <= 1 {
        values
    } else if values[0] == values[1] {
        dedup_sorted(values.drop_first())
    } else {
        seq![values[0]] + dedup_sorted(values.drop_first())
    }
}

pub proof fn lemma_dedup_sorted_properties(values: Seq<(Seq<u8>, Seq<u8>)>)
    requires
        sorted_by(values, leq_pair_fn()),
    ensures
        sorted_by(dedup_sorted(values), leq_pair_fn()),
        dedup_sorted(values).no_duplicates(),
        dedup_sorted(values).to_set() =~= values.to_set(),
        dedup_sorted(values).len() <= values.len(),
    decreases values.len(),
{
    if values.len() <= 1 {
        if values.len() == 1 {
            assert(dedup_sorted(values).no_duplicates());
        }
    } else {
        let rest = values.drop_first();
        assert(sorted_by(rest, leq_pair_fn()));
        lemma_dedup_sorted_properties(rest);
        if values[0] == values[1] {
            assert(values.to_set() =~= rest.to_set()) by {
                broadcast use Seq::to_set_ensures, lemma_seq_contains, group_set_properties;

                assert(rest[0] == values[0]);
            };
        } else {
            let deduped = dedup_sorted(rest);
            assert(dedup_sorted(values) =~= seq![values[0]] + deduped);
            // every surviving member of rest is at or above values[0]
            assert forall|v| deduped.to_set().contains(v) implies leq_pair(values[0], v) by {
                broadcast use Seq::to_set_ensures, lemma_seq_contains;

                assert(rest.to_set().contains(v));
                let k = choose|k: int| 0 <= k < rest.len() && rest[k] == v;
                assert(values[k + 1] == v);
                assert(leq_pair_fn()(values[0], values[k + 1]));
            };
            // and values[0] itself does not survive
            assert(!deduped.to_set().contains(values[0])) by {
                broadcast use Seq::to_set_ensures, lemma_seq_contains;

                if deduped.to_set().contains(values[0]) {
                    assert(rest.to_set().contains(values[0]));
                    let k = choose|k: int| 0 <= k < rest.len() && rest[k] == values[0];
                    lemma_sorted_repeat_adjacent(values, 0, k + 1);
                }
            };
            assert(dedup_sorted(values).no_duplicates()) by {
                broadcast use Seq::to_set_ensures, lemma_seq_contains;

                assert forall|j2: int| 0 <= j2 < deduped.len() implies deduped[j2] != values[0] by {
                    assert(deduped.to_set().contains(deduped[j2]));
                };
                lemma_no_dup_in_concat(seq![values[0]], deduped);
            };
            assert forall|x: int, y: int| 0 <= x < y < dedup_sorted(values).len() implies
                leq_pair(dedup_sorted(values)[x], dedup_sorted(values)[y]) by {
                broadcast use Seq::to_set_ensures;

                if x == 0 {
                    assert(dedup_sorted(values)[y] == deduped[y - 1]);
                    assert(deduped.to_set().contains(deduped[y - 1]));
                } else {
                    assert(dedup_sorted(values)[x] == deduped[x - 1]);
                    assert(dedup_sorted(values)[y] == deduped[y - 1]);
                    assert(leq_pair_fn()(deduped[x - 1], deduped[y - 1]));
                }
            };
            assert forall|x: int, y: int| 0 <= x < y < dedup_sorted(values).len() implies #[trigger]
                leq_pair_fn()(dedup_sorted(values)[x], dedup_sorted(values)[y]) by {
            };
            assert(deduped.to_set() =~= rest.to_set());
            assert(dedup_sorted(values).to_set() =~= (seq![values[0]] + deduped).to_set());
            assert(values =~= seq![values[0]] + rest);
            assert((seq![values[0]] + deduped).to_set() =~= values.to_set()) by {
                broadcast use Seq::to_set_ensures, lemma_seq_contains, group_set_properties;

                assert forall|v| (seq![values[0]] + deduped).to_set().contains(v) implies
                    values.to_set().contains(v) by {
                    if v == values[0] {
                        assert(values[0] == v);
                    } else {
                        assert(deduped.to_set().contains(v));
                        let k = choose|k: int| 0 <= k < rest.len() && rest[k] == v;
                        assert(values[k + 1] == v);
                    }
                };
                assert forall|v| values.to_set().contains(v) implies (seq![values[0]]
                    + deduped).to_set().contains(v) by {
                    let k = choose|k: int| 0 <= k < values.len() && values[k] == v;
                    if k == 0 {
                        assert((seq![values[0]] + deduped)[0] == v);
                        assert((seq![values[0]] + deduped).contains(v));
                    } else {
                        assert(rest[k - 1] == v);
                        assert(rest.to_set().contains(v));
                        assert(deduped.to_set().contains(v));
                        let m = choose|m: int| 0 <= m < deduped.len() && deduped[m] == v;
                        assert((seq![values[0]] + deduped)[m + 1] == v);
                        assert((seq![values[0]] + deduped).contains(v));
                    }
                };
            };
        }
    }
}

/// In a sorted sequence, any repetition of element `from` occupies the
/// adjacent position.
pub proof fn lemma_sorted_repeat_adjacent(values: Seq<(Seq<u8>, Seq<u8>)>, from: int, at: int)
    requires
        sorted_by(values, leq_pair_fn()),
        0 <= from < values.len(),
        from < at < values.len(),
        values[at] == values[from],
    ensures
        values[from + 1] == values[from],
{
    assert(leq_pair_fn()(values[from], values[from + 1]));
    if from + 1 < at {
        assert(leq_pair_fn()(values[from + 1], values[at]));
    } else {
        lemma_leq_pair_reflexive(values[from + 1]);
    }
    lemma_leq_pair_reflexive(values[from]);
    lemma_leq_pair_antisymmetric(values[from], values[from + 1]);
}

/// The canonical form of an unordered collection: the unique sorted,
/// duplicate-free presentation. Module, label and import declaration
/// order cannot change it.
pub open spec fn canonical_form(values: Seq<(Seq<u8>, Seq<u8>)>) -> Seq<(Seq<u8>, Seq<u8>)> {
    dedup_sorted(values.sort_by(leq_pair_fn()))
}

pub proof fn lemma_canonical_form_properties(values: Seq<(Seq<u8>, Seq<u8>)>)
    ensures
        sorted_by(canonical_form(values), leq_pair_fn()),
        canonical_form(values).no_duplicates(),
        canonical_form(values).to_set() =~= values.to_set(),
        canonical_form(values).len() <= values.len(),
{
    lemma_leq_pair_total_ordering();
    values.lemma_sort_by_ensures(leq_pair_fn());
    let sorted = values.sort_by(leq_pair_fn());
    assert(sorted.to_multiset() =~= values.to_multiset());
    assert(sorted.to_set() =~= values.to_set()) by {
        broadcast use group_to_multiset_ensures, Seq::to_set_ensures, group_set_properties,
            group_multiset_axioms;

        assert forall|v| sorted.to_set().contains(v) == values.to_set().contains(v) by {
            assert(sorted.to_multiset().contains(v) == values.to_multiset().contains(v));
        };
    };
    assert(sorted.len() == values.len()) by {
        broadcast use group_to_multiset_ensures, group_multiset_axioms;

        assert(sorted.to_multiset().len() == values.to_multiset().len());
    };
    lemma_dedup_sorted_properties(sorted);
    assert(canonical_form(values).to_set() =~= values.to_set());
    assert(canonical_form(values).len() <= sorted.len());
}

/// Canonical-form uniqueness: presentations with the same members —
/// any module/label/import order, any duplicate spelling — normalize
/// to one sequence.
pub proof fn lemma_canonical_form_unique(a: Seq<(Seq<u8>, Seq<u8>)>, b: Seq<(Seq<u8>, Seq<u8>)>)
    requires
        a.to_set() == b.to_set(),
    ensures
        canonical_form(a) == canonical_form(b),
{
    lemma_canonical_form_properties(a);
    lemma_canonical_form_properties(b);
    lemma_sorted_nodup_unique(canonical_form(a), canonical_form(b));
}

/// Two sorted duplicate-free sequences with the same members are equal.
pub proof fn lemma_sorted_nodup_unique(x: Seq<(Seq<u8>, Seq<u8>)>, y: Seq<(Seq<u8>, Seq<u8>)>)
    requires
        sorted_by(x, leq_pair_fn()),
        sorted_by(y, leq_pair_fn()),
        x.no_duplicates(),
        y.no_duplicates(),
        x.to_set() == y.to_set(),
    ensures
        x == y,
{
    x.lemma_multiset_has_no_duplicates();
    y.lemma_multiset_has_no_duplicates();
    assert(x.to_multiset() =~= y.to_multiset()) by {
        broadcast use group_to_multiset_ensures, Seq::to_set_ensures, group_multiset_axioms;

        assert forall|v: (Seq<u8>, Seq<u8>)| x.to_multiset().count(v) == y.to_multiset().count(
            v,
        ) by {
            assert(x.to_set().contains(v) == y.to_set().contains(v));
            assert(x.to_multiset().contains(v) == x.contains(v));
            assert(x.to_set().contains(v) == x.contains(v));
            assert(y.to_multiset().contains(v) == y.contains(v));
            assert(y.to_set().contains(v) == y.contains(v));
        };
    };
    lemma_leq_pair_total_ordering();
    lemma_sorted_unique(x, y, leq_pair_fn());
}

/// Representation relation: any production canonicalizer result
/// (sorted, duplicate-free, same members — exactly the
/// `canonical_entries` postconditions) IS the spec canonical form of
/// its input. The shipped encoder therefore writes
/// `unordered_frames(entry_pairs(input))`, proved canonical.
pub proof fn lemma_canonical_entries_match<V>(
    values: Seq<(&[u8], &[u8], V)>,
    result: Seq<(&[u8], &[u8], V)>,
)
    requires
        sorted_by(entry_pairs(result), leq_pair_fn()),
        entry_pairs(result).no_duplicates(),
        entry_pairs(result).to_set() =~= entry_pairs(values).to_set(),
    ensures
        entry_pairs(result) == canonical_form(entry_pairs(values)),
{
    lemma_canonical_form_properties(entry_pairs(values));
    lemma_sorted_nodup_unique(entry_pairs(result), canonical_form(entry_pairs(values)));
}

/* ----------------------------------------------------------------
 * Collection encodings and the equivalence/invalidation relation
 * -------------------------------------------------------------- */

/// The frame list of a canonical entry sequence: each entry contributes
/// its two fields in order (production: scope+key, or key+value).
pub open spec fn entry_frames(entries: Seq<(Seq<u8>, Seq<u8>)>) -> Seq<Seq<u8>>
    decreases entries.len(),
{
    if entries.len() == 0 {
        Seq::empty()
    } else {
        seq![entries[0].0, entries[0].1] + entry_frames(entries.drop_first())
    }
}

/// The frame list of an entry sequence is exactly two frames per entry.
pub proof fn lemma_entry_frames_len(entries: Seq<(Seq<u8>, Seq<u8>)>)
    ensures
        entry_frames(entries).len() == 2 * entries.len(),
    decreases entries.len(),
{
    if entries.len() > 0 {
        lemma_entry_frames_len(entries.drop_first());
        assert(entry_frames(entries) =~= seq![entries[0].0, entries[0].1] + entry_frames(
            entries.drop_first(),
        ));
    }
}

/// The frame list of an entry sequence lists each entry in order.
pub proof fn lemma_entry_frames_index(entries: Seq<(Seq<u8>, Seq<u8>)>, i: int)
    requires
        0 <= i < entries.len(),
    ensures
        entry_frames(entries)[2 * i] == entries[i].0,
        entry_frames(entries)[2 * i + 1] == entries[i].1,
    decreases i,
{
    broadcast use vstd::seq::lemma_seq_add_index2;

    let head: Seq<Seq<u8>> = seq![entries[0].0, entries[0].1];
    let rest = entry_frames(entries.drop_first());
    assert(entry_frames(entries) =~= head + rest);
    if i > 0 {
        lemma_entry_frames_index(entries.drop_first(), i - 1);
        lemma_entry_frames_len(entries.drop_first());
        assert(rest.len() == 2 * (entries.len() - 1));
        assert(head.len() == 2);
        assert(2 <= 2 * i && 2 * i + 1 < 2 + rest.len());
        assert((head + rest)[2 * i] == rest[2 * i - 2]);
        assert((head + rest)[2 * i + 1] == rest[2 * i - 1]);
        assert(entries.drop_first()[i - 1] == entries[i]);
    }
}

/// Members of the canonical form are members of the presentation, so
/// presentation-level byte bounds bound the canonical frames.
pub proof fn lemma_canonical_members_bounded(values: Seq<(Seq<u8>, Seq<u8>)>, m: int)
    requires
        forall|i: int| 0 <= i < values.len() ==> #[trigger] values[i].0.len() <= u64::MAX as int
            && values[i].1.len() <= u64::MAX as int,
        0 <= m < canonical_form(values).len(),
    ensures
        canonical_form(values)[m].0.len() <= u64::MAX as int,
        canonical_form(values)[m].1.len() <= u64::MAX as int,
{
    broadcast use Seq::to_set_ensures, lemma_seq_contains;

    lemma_canonical_form_properties(values);
    let member = canonical_form(values)[m];
    assert(canonical_form(values).to_set().contains(member));
    assert(values.to_set().contains(member));
    let k = choose|k: int| 0 <= k < values.len() && #[trigger] values[k] == member;
}

pub proof fn lemma_entry_frames_injective(x: Seq<(Seq<u8>, Seq<u8>)>, y: Seq<(Seq<u8>, Seq<u8>)>)
    requires
        entry_frames(x) == entry_frames(y),
    ensures
        x == y,
    decreases x.len(), y.len(),
{
    if x.len() == 0 || y.len() == 0 {
        if x.len() > 0 {
            assert(entry_frames(x) =~= seq![x[0].0, x[0].1] + entry_frames(x.drop_first()));
        }
        if y.len() > 0 {
            assert(entry_frames(y) =~= seq![y[0].0, y[0].1] + entry_frames(y.drop_first()));
        }
    } else {
        let fx = entry_frames(x);
        let fy = entry_frames(y);
        assert(fx =~= seq![x[0].0, x[0].1] + entry_frames(x.drop_first()));
        assert(fy =~= seq![y[0].0, y[0].1] + entry_frames(y.drop_first()));
        assert(fx[0] == x[0].0 && fx[1] == x[0].1);
        assert(fy[0] == y[0].0 && fy[1] == y[0].1);
        assert(x[0] == y[0]);
        assert(entry_frames(x.drop_first()) =~= fx.subrange(2, fx.len() as int));
        assert(entry_frames(y.drop_first()) =~= fy.subrange(2, fy.len() as int));
        lemma_entry_frames_injective(x.drop_first(), y.drop_first());
        assert(x =~= seq![x[0]] + x.drop_first());
        assert(y =~= seq![y[0]] + y.drop_first());
    }
}

/// Frames of an unordered collection: a count frame, then each
/// canonical entry's fields (production: unordered `strings`, mutation
/// `locks`, environment maps).
pub open spec fn unordered_frames(values: Seq<(Seq<u8>, Seq<u8>)>) -> Seq<Seq<u8>> {
    seq![le64(canonical_form(values).len() as u64)] + entry_frames(canonical_form(values))
}

/// Frames of an ordered collection: a count frame, then each element in
/// declared order (production: argv, run-bash options, steps).
pub open spec fn ordered_frames(values: Seq<Seq<u8>>) -> Seq<Seq<u8>> {
    seq![le64(values.len() as u64)] + values
}

/// Label/module/import-order invariance: presentations with the same
/// members encode to the same frames.
pub proof fn lemma_unordered_presentation_invariant(
    a: Seq<(Seq<u8>, Seq<u8>)>,
    b: Seq<(Seq<u8>, Seq<u8>)>,
)
    requires
        a.to_set() == b.to_set(),
    ensures
        unordered_frames(a) == unordered_frames(b),
{
    lemma_canonical_form_unique(a, b);
}

/// Unordered invalidation: equal encodings imply equal members — no
/// distinct set of entries ever aliases one canonical encoding.
pub proof fn lemma_unordered_content_injective(a: Seq<(Seq<u8>, Seq<u8>)>, b: Seq<(
    Seq<u8>,
    Seq<u8>,
)>)
    requires
        forall|i: int| 0 <= i < a.len() ==> #[trigger] a[i].0.len() <= u64::MAX as int
            && a[i].1.len() <= u64::MAX as int,
        forall|i: int| 0 <= i < b.len() ==> #[trigger] b[i].0.len() <= u64::MAX as int
            && b[i].1.len() <= u64::MAX as int,
        encode_frames(unordered_frames(a)) == encode_frames(unordered_frames(b)),
    ensures
        a.to_set() == b.to_set(),
{
    lemma_canonical_form_properties(a);
    lemma_canonical_form_properties(b);
    lemma_entry_frames_len(canonical_form(a));
    lemma_entry_frames_len(canonical_form(b));
    assert(unordered_frames(a).len() == 1 + entry_frames(canonical_form(a)).len());
    assert(unordered_frames(b).len() == 1 + entry_frames(canonical_form(b)).len());
    assert forall|i: int| 0 <= i < unordered_frames(a).len() implies #[trigger]
        unordered_frames(a)[i].len() <= u64::MAX as int by {
        if i > 0 {
            let m = (i - 1) / 2;
            lemma_canonical_members_bounded(a, m);
            lemma_entry_frames_index(canonical_form(a), m);
            assert(0 <= m < canonical_form(a).len());
            assert(2 * m == i - 1 || 2 * m + 1 == i - 1);
            assert(entry_frames(canonical_form(a))[2 * m] == canonical_form(a)[m].0);
            assert(entry_frames(canonical_form(a))[2 * m + 1] == canonical_form(a)[m].1);
            assert(unordered_frames(a)[i] == entry_frames(canonical_form(a))[i - 1]);
        }
    };
    assert forall|i: int| 0 <= i < unordered_frames(b).len() implies #[trigger]
        unordered_frames(b)[i].len() <= u64::MAX as int by {
        if i > 0 {
            let m = (i - 1) / 2;
            lemma_canonical_members_bounded(b, m);
            lemma_entry_frames_index(canonical_form(b), m);
            assert(0 <= m < canonical_form(b).len());
            assert(2 * m == i - 1 || 2 * m + 1 == i - 1);
            assert(entry_frames(canonical_form(b))[2 * m] == canonical_form(b)[m].0);
            assert(entry_frames(canonical_form(b))[2 * m + 1] == canonical_form(b)[m].1);
            assert(unordered_frames(b)[i] == entry_frames(canonical_form(b))[i - 1]);
        }
    };
    lemma_encode_frames_injective(unordered_frames(a), unordered_frames(b));
    assert(unordered_frames(a).drop_first() =~= entry_frames(canonical_form(a)));
    assert(unordered_frames(b).drop_first() =~= entry_frames(canonical_form(b)));
    lemma_entry_frames_injective(canonical_form(a), canonical_form(b));
}

/// Reordering sensitivity: ordered collections are injected element by
/// element — a reordered distinct list never aliases the encoding.
pub proof fn lemma_ordered_sensitive(a: Seq<Seq<u8>>, b: Seq<Seq<u8>>)
    requires
        forall|i: int| 0 <= i < a.len() ==> #[trigger] a[i].len() <= u64::MAX as int,
        forall|i: int| 0 <= i < b.len() ==> #[trigger] b[i].len() <= u64::MAX as int,
        encode_frames(ordered_frames(a)) == encode_frames(ordered_frames(b)),
    ensures
        a == b,
{
    lemma_encode_frames_injective(ordered_frames(a), ordered_frames(b));
    assert(ordered_frames(a).drop_first() =~= a);
    assert(ordered_frames(b).drop_first() =~= b);
}

/// An environment presentation is keyed: no key arrives twice (TS
/// `asEnv` keys an object; Rust decodes a `BTreeMap`).
pub open spec fn unique_keys(entries: Seq<(Seq<u8>, Seq<u8>)>) -> bool {
    forall|i: int, j: int|
        0 <= i < entries.len() && 0 <= j < entries.len() && entries[i].0 == entries[j].0 ==> i
            == j
}

/// Fluent ≡ object (A1-03): the immutable fluent builder accumulates
/// entries in call order, the object form in literal order; both are
/// unique-keyed presentations of one environment, so both normalize to
/// byte-identical frames regardless of arrival order.
pub proof fn lemma_fluent_object_equivalence(
    fluent: Seq<(Seq<u8>, Seq<u8>)>,
    object: Seq<(Seq<u8>, Seq<u8>)>,
)
    requires
        unique_keys(fluent),
        unique_keys(object),
        fluent.to_set() == object.to_set(),
    ensures
        unordered_frames(fluent) == unordered_frames(object),
{
    lemma_unordered_presentation_invariant(fluent, object);
}

/// The converse boundary: equal environment encodings carry equal
/// entries — an environment content change always invalidates identity.
pub proof fn lemma_environment_change_invalidates(
    fluent: Seq<(Seq<u8>, Seq<u8>)>,
    object: Seq<(Seq<u8>, Seq<u8>)>,
)
    requires
        forall|i: int| 0 <= i < fluent.len() ==> #[trigger] fluent[i].0.len()
            <= u64::MAX as int && fluent[i].1.len() <= u64::MAX as int,
        forall|i: int| 0 <= i < object.len() ==> #[trigger] object[i].0.len()
            <= u64::MAX as int && object[i].1.len() <= u64::MAX as int,
        encode_frames(unordered_frames(fluent)) == encode_frames(unordered_frames(object)),
    ensures
        fluent.to_set() == object.to_set(),
{
    lemma_unordered_content_injective(fluent, object);
}

/* ----------------------------------------------------------------
 * Semantic vs provenance identity boundary
 * -------------------------------------------------------------- */

/// A declaration view: semantic frames (hashed) alongside provenance
/// (source spans, dedent line maps, labels — diagnostic only).
pub struct DeclarationView {
    pub semantics: Seq<Seq<u8>>,
    pub provenance: Seq<Seq<u8>>,
}

/// The identity encoding of a declaration: its semantic frames, framed
/// and concatenated. Provenance is not an argument — it can never enter
/// identity (A1-04: diagnostic-only changes preserve admitted recipe
/// identity).
pub open spec fn declaration_encoding(view: DeclarationView) -> Seq<u8> {
    encode_frames(view.semantics)
}

/// Diagnostic-only edits preserve identity, by construction.
pub proof fn lemma_provenance_cannot_change_identity(a: DeclarationView, b: DeclarationView)
    ensures
        a.semantics == b.semantics ==> declaration_encoding(a) == declaration_encoding(b),
{
}

/// Semantic edits always invalidate identity: equal encodings carry
/// equal semantic frames. Distinct semantic content therefore hashes
/// differently unless SHA-256 itself collides (the trusted adapter).
pub proof fn lemma_semantic_change_invalidates_identity(a: DeclarationView, b: DeclarationView)
    requires
        forall|i: int| 0 <= i < a.semantics.len() ==> #[trigger] a.semantics[i].len() <= u64::MAX as int,
        forall|i: int| 0 <= i < b.semantics.len() ==> #[trigger] b.semantics[i].len() <= u64::MAX as int,
    ensures
        declaration_encoding(a) == declaration_encoding(b) ==> a.semantics == b.semantics,
{
    if declaration_encoding(a) == declaration_encoding(b) {
        lemma_encode_frames_injective(a.semantics, b.semantics);
    }
}

}
