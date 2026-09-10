//! The merge splice kernel (0046's successor, plan/0047): byte-exact
//! replacement of owned spans in a hosting text. This is the
//! byte-integrity heart of merge mode — the contract is stated over
//! the actual input/output bytes (handoff §5.5), never over an
//! abstract "foreign text preserved" flag.
//!
//! `ManagedBlockSet` (gripsack-exec) supplies the spans from its
//! parser and converts back through `String::from_utf8` — span
//! alignment on UTF-8 boundaries is the PARSER's obligation, not the
//! kernel's.

use vstd::prelude::*;
// plain-cargo shim builds see no use of the seq lemmas; the
// `broadcast use` below (erased outside verification) needs them
#[allow(unused_imports)]
use vstd::seq_lib::*;

verus! {

broadcast use group_seq_properties;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

pub open spec fn valid_spans(text: Seq<u8>, spans: Seq<Span>) -> bool {
    &&& forall|i: int| 0 <= i < spans.len() ==>
            #[trigger] spans[i].start <= spans[i].end && spans[i].end <= text.len()
    &&& forall|i: int| 0 <= i < spans.len() - 1 ==>
            spans[i].end <= spans[i + 1].start
}

/// The suffix of the output still to be produced: gap before span i,
/// the replacement at the first span, then recurse.
pub open spec fn spec_splice_rest(text: Seq<u8>, spans: Seq<Span>, replacement: Seq<u8>, i: int, cursor: int) -> Seq<u8>
    decreases spans.len() - i,
{
    if i >= spans.len() {
        text.subrange(cursor, text.len() as int)
    } else {
        text.subrange(cursor, spans[i].start as int)
            + (if i == 0 { replacement } else { Seq::empty() })
            + spec_splice_rest(text, spans, replacement, i + 1, spans[i].end as int)
    }
}

pub open spec fn spec_splice(text: Seq<u8>, spans: Seq<Span>, replacement: Seq<u8>) -> Seq<u8> {
    spec_splice_rest(text, spans, replacement, 0, 0)
}

pub fn splice_bytes(text: &[u8], spans: &[Span], replacement: &[u8]) -> (result: Vec<u8>)
    requires
        valid_spans(text@, spans@),
    ensures
        result@ == spec_splice(text@, spans@, replacement@),
{
    let mut out: Vec<u8> = Vec::new();
    let mut cursor: usize = 0;
    let mut i: usize = 0;
    while i < spans.len()
        invariant
            i <= spans.len(),
            valid_spans(text@, spans@),
            i > 0 ==> cursor == spans@[i - 1].end,
            i == 0 ==> cursor == 0,
            cursor <= text@.len(),
            out@ + spec_splice_rest(text@, spans@, replacement@, i as int, cursor as int)
                == spec_splice(text@, spans@, replacement@),
        decreases spans.len() - i,
    {
        out.extend_from_slice(&text[cursor..spans[i].start]);
        if i == 0 {
            out.extend_from_slice(replacement);
        }
        cursor = spans[i].end;
        i += 1;
    }
    out.extend_from_slice(&text[cursor..]);
    out
}

/// The gaps-only recursion: text between span ends and next starts,
/// ending with the tail past the last span.
pub open spec fn spec_gaps(text: Seq<u8>, spans: Seq<Span>, i: int) -> Seq<u8>
    decreases spans.len() - i,
{
    let next_start = if i + 1 < spans.len() { spans[i + 1].start as int } else { text.len() as int };
    let gap = text.subrange(spans[i].end as int, next_start);
    if i + 1 < spans.len() {
        gap + spec_gaps(text, spans, i + 1)
    } else {
        gap
    }
}

/// rest(i) with the correct cursor is the pre-gap, maybe the
/// replacement, then the gaps recursion.
pub proof fn lemma_rest_is_gaps(text: Seq<u8>, spans: Seq<Span>, replacement: Seq<u8>, i: int, cursor: int)
    requires
        valid_spans(text, spans),
        0 <= i < spans.len(),
        i == 0 ==> cursor == 0,
        i > 0 ==> cursor == spans[i - 1].end,
    ensures
        spec_splice_rest(text, spans, replacement, i, cursor)
            == text.subrange(cursor, spans[i].start as int)
                + (if i == 0 { replacement } else { Seq::empty() })
                + spec_gaps(text, spans, i),
    decreases spans.len() - i,
{
    if i + 1 < spans.len() {
        lemma_rest_is_gaps(text, spans, replacement, i + 1, spans[i].end as int);
    } else {
        // the last gap runs to the end of the text
        assert(spec_gaps(text, spans, i)
            == text.subrange(spans[i].end as int, text.len() as int));
    }
    // unfold one recursion step explicitly (fuel-safe), then finish
    let head = text.subrange(cursor, spans[i].start as int);
    let mid = if i == 0 { replacement } else { Seq::empty() };
    assert(spec_splice_rest(text, spans, replacement, i, cursor)
        == head + mid + spec_splice_rest(text, spans, replacement, i + 1, spans[i].end as int));
    assert(spec_gaps(text, spans, i)
        == text.subrange(spans[i].end as int, if i + 1 < spans.len() {
            spans[i + 1].start as int
        } else {
            text.len() as int
        }) + (if i + 1 < spans.len() {
            spec_gaps(text, spans, i + 1)
        } else {
            Seq::empty()
        }));
    assert_seqs_equal!(
        spec_splice_rest(text, spans, replacement, i, cursor),
        head + mid + spec_gaps(text, spans, i)
    );
}

/// The canonical form: head ++ replacement ++ gaps-and-tail.
pub proof fn lemma_splice_canonical(text: Seq<u8>, spans: Seq<Span>, replacement: Seq<u8>)
    requires
        valid_spans(text, spans),
        spans.len() > 0,
    ensures
        spec_splice(text, spans, replacement)
            == text.subrange(0, spans[0].start as int) + replacement + spec_gaps(text, spans, 0),
{
    lemma_rest_is_gaps(text, spans, replacement, 0, 0);
}

/// Identity: one span replaced by its own content is a no-op.
pub proof fn lemma_splice_identity(text: Seq<u8>, span: Span)
    requires
        valid_spans(text, seq![span]),
    ensures
        spec_splice(text, seq![span], text.subrange(span.start as int, span.end as int)) == text,
{
    let (a, b, n) = (span.start as int, span.end as int, text.len() as int);
    lemma_splice_canonical(text, seq![span], text.subrange(a, b));
    // one span: the gaps recursion is exactly the tail
    assert(spec_gaps(text, seq![span], 0) == text.subrange(b, n));
    // head ++ own content ++ tail == the whole text, pointwise
    assert_seqs_equal!(
        text.subrange(0, a) + text.subrange(a, b) + text.subrange(b, n),
        text
    );
}

/// The gaps recursion always ends with the tail past the last span.
pub proof fn lemma_gaps_end_with_tail(text: Seq<u8>, spans: Seq<Span>, i: int)
    requires
        valid_spans(text, spans),
        0 <= i < spans.len(),
    ensures
        ({
            let tail = text.subrange(spans[spans.len() - 1].end as int, text.len() as int);
            let gaps = spec_gaps(text, spans, i);
            gaps.len() >= tail.len()
                && gaps.subrange(gaps.len() - tail.len(), gaps.len() as int) == tail
        }),
    decreases spans.len() - i,
{
    if i + 1 < spans.len() {
        lemma_gaps_end_with_tail(text, spans, i + 1);
        let gaps = spec_gaps(text, spans, i);
        let rest = spec_gaps(text, spans, i + 1);
        let tail = text.subrange(spans[spans.len() - 1].end as int, text.len() as int);
        // gaps == gap_i ++ rest; lengths add; rest is long enough
        assert(gaps.len() == text.subrange(spans[i].end as int, spans[i + 1].start as int).len()
            + rest.len());
        assert(rest.len() >= tail.len());
        assert_seqs_equal!(
            gaps.subrange(gaps.len() - tail.len(), gaps.len() as int),
            tail
        );
    }
}

/// Foreign head and tail survive verbatim.
pub proof fn lemma_splice_preserves_edges(text: Seq<u8>, spans: Seq<Span>, replacement: Seq<u8>)
    requires
        valid_spans(text, spans),
        spans.len() > 0,
    ensures
        // the head before the first block is the output's prefix
        spec_splice(text, spans, replacement).subrange(0, spans[0].start as int)
            == text.subrange(0, spans[0].start as int),
        // the tail past the last block is the output's suffix
        ({
            let out = spec_splice(text, spans, replacement);
            let tail = text.subrange(spans[spans.len() - 1].end as int, text.len() as int);
            out.subrange(out.len() - tail.len(), out.len() as int) == tail
        }),
{
    lemma_splice_canonical(text, spans, replacement);
    lemma_gaps_end_with_tail(text, spans, 0);
    let out = spec_splice(text, spans, replacement);
    let tail = text.subrange(spans[spans.len() - 1].end as int, text.len() as int);
    let head = text.subrange(0, spans[0].start as int);
    let gaps = spec_gaps(text, spans, 0);
    // out == head ++ replacement ++ gaps; prefix of the first piece
    assert_seqs_equal!(out.subrange(0, head.len() as int), head);
    // out == head ++ replacement ++ gaps, gaps ends with the tail
    // and is long enough; so out ends with the tail
    assert(out.len() == head.len() + replacement.len() + gaps.len());
    assert(gaps.len() >= tail.len());
    assert_seqs_equal!(out.subrange(out.len() - tail.len(), out.len() as int), tail);
}

}
