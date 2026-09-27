//! The production managed-block line walk and range state machine. Lexical
//! marker recognition is supplied by exec; range safety holds for every
//! returned classification, not an assumed property of that recognizer.
use super::Span;
use vstd::prelude::*;
#[allow(unused_imports)]
use vstd::{string::StringSliceAdditionalSpecFns, utf8::*};

verus! {

fn span_text(text: &str, start: usize, end: usize) -> (result: &str)
    requires
        start <= end <= text.spec_bytes().len(),
        text.is_char_boundary(start),
        text.is_char_boundary(end),
    ensures
        result.spec_bytes() == text.spec_bytes().subrange(start as int, end as int),
{
    let (prefix, _) = text.split_at(end);
    proof {
        vstd::utf8::encode_utf8_valid_utf8(text@);
        vstd::utf8::encode_utf8_valid_utf8(prefix@);
        if start < end {
            vstd::utf8::is_char_boundary_iff_not_is_continuation_byte(text.spec_bytes(), start as int);
            vstd::utf8::is_char_boundary_iff_not_is_continuation_byte(prefix.spec_bytes(), start as int);
        } else {
            vstd::utf8::is_char_boundary_start_end_of_seq(prefix.spec_bytes());
        }
    }
    let (_, result) = prefix.split_at(start);
    result
}

proof fn lemma_ascii_successor(bytes: Seq<u8>, position: int)
    requires
        valid_utf8(bytes),
        0 <= position < bytes.len(),
        bytes[position] < 0x80,
    ensures
        is_char_boundary(bytes, position + 1),
{
    broadcast use vstd::seq::group_seq_lemmas;
    is_char_boundary_iff_not_is_continuation_byte(bytes, position);
    valid_utf8_split(bytes, position);
    let suffix = bytes.subrange(position, bytes.len() as int);
    reveal_with_fuel(valid_utf8, 2);
    assert(pop_first_scalar(suffix) =~= bytes.subrange(position + 1, bytes.len() as int));
    if position + 1 < bytes.len() {
        let rest = bytes.subrange(position + 1, bytes.len() as int);
        assert(valid_first_scalar(rest));
        assert(!is_continuation_byte(bytes[position + 1]));
        is_char_boundary_iff_not_is_continuation_byte(bytes, position + 1);
    } else {
        is_char_boundary_start_end_of_seq(bytes);
    }
}

fn next_line_end(text: &str, start: usize) -> (end: usize)
    requires
        start < text.spec_bytes().len(),
    ensures
        start < end <= text.spec_bytes().len(),
        text.is_char_boundary(end),
        end < text.spec_bytes().len() ==> text.spec_bytes()[end as int - 1] == b'\n',
        forall|i: int| start <= i < end - 1 ==> #[trigger] text.spec_bytes()[i] != b'\n',
{
    let bytes = text.as_bytes();
    let mut end = start;
    while end < bytes.len() && bytes[end] != b'\n'
        invariant
            start <= end <= bytes.len(),
            bytes@ == text.spec_bytes(),
            forall|i: int| start <= i < end ==> #[trigger] bytes@[i] != b'\n',
        decreases bytes.len() - end,
    {
        end += 1;
    }
    proof { encode_utf8_valid_utf8(text@); }
    if end < bytes.len() {
        proof { lemma_ascii_successor(bytes@, end as int); }
        end += 1;
    } else {
        proof { is_char_boundary_start_end_of_seq(bytes@); }
    }
    end
}

#[derive(Debug)]
pub struct ParsedBlock<'a> {
    pub span: Span,
    pub content: &'a str,
    pub recorded_hash: &'a str,
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub enum BodyPosition { First, Later }

#[derive(Debug)]
pub enum LineKind<'a> {
    Open { module: &'a str, hash: &'a str, mode: Option<u32> },
    Close(&'a str),
    Content,
    LegacyHeader,
}

#[derive(Debug)]
pub enum ScanFailure<E> {
    Metadata(E),
    NestedOpening { line: usize },
    MissingOpening { line: usize },
    DifferentModule { line: usize },
    MissingClosing { line: usize },
}

struct Opening<'a> {
    module: &'a str,
    hash: &'a str,
    mode: Option<u32>,
    start: usize,
    content_start: usize,
    line: usize,
    body_position: BodyPosition,
}

pub open spec fn valid_block_spans(text: Seq<u8>, blocks: Seq<ParsedBlock>) -> bool {
    &&& forall|i: int| 0 <= i < blocks.len() ==> {
        &&& #[trigger] blocks[i].span.start < blocks[i].span.end <= text.len()
        &&& is_char_boundary(text, blocks[i].span.start as int)
        &&& is_char_boundary(text, blocks[i].span.end as int)
    }
    &&& forall|i: int| 0 <= i < blocks.len() - 1 ==>
        blocks[i].span.end <= blocks[i + 1].span.start
}

/// Exactly the span projection used by the exec wrapper at splice admission.
pub open spec fn projected_spans(blocks: Seq<ParsedBlock>) -> Seq<Span> {
    Seq::new(blocks.len(), |i: int| blocks[i].span)
}

pub proof fn scan_admits_splice(text: Seq<u8>, blocks: Seq<ParsedBlock>)
    requires valid_block_spans(text, blocks),
    ensures super::valid_spans(text, projected_spans(blocks)),
{
    assert forall|i: int| 0 <= i < blocks.len() implies
        #[trigger] projected_spans(blocks)[i].start <= projected_spans(blocks)[i].end
        && projected_spans(blocks)[i].end <= text.len() by {
        assert(blocks[i].span.start < blocks[i].span.end <= text.len());
        assert(projected_spans(blocks)[i] == blocks[i].span);
    };
    assert forall|i: int| 0 <= i < blocks.len() - 1 implies
        #[trigger] projected_spans(blocks)[i].end <= projected_spans(blocks)[i + 1].start by {};
}

pub fn scan<'a, E, F: Fn(&'a str, usize, Option<BodyPosition>) -> Result<LineKind<'a>, E>>(
    text: &'a str,
    wanted: &str,
    classifier: F,
) -> (result: Result<Vec<ParsedBlock<'a>>, ScanFailure<E>>)
    requires
        forall|line: &'a str, number: usize, position: Option<BodyPosition>|
            classifier.requires((line, number, position)),
    ensures
        match result {
            Ok(blocks) => valid_block_spans(text.spec_bytes(), blocks@)
                && super::valid_spans(text.spec_bytes(), projected_spans(blocks@)),
            Err(_) => true,
        },
{
    let mut blocks: Vec<ParsedBlock<'a>> = Vec::new();
    let mut opening: Option<Opening<'a>> = None;
    let mut offset: usize = 0;
    let mut number: usize = 0;
    proof {
        encode_utf8_valid_utf8(text@);
        is_char_boundary_start_end_of_seq(text.spec_bytes());
    }
    while offset < text.len()
        invariant
            offset <= text.spec_bytes().len(),
            blocks.len() <= number <= offset,
            text.is_char_boundary(offset),
            valid_utf8(text.spec_bytes()),
            valid_block_spans(text.spec_bytes(), blocks@),
            forall|i: int| 0 <= i < blocks.len() ==> #[trigger] blocks@[i].span.end <= offset,
            forall|line: &'a str, number: usize, position: Option<BodyPosition>|
                classifier.requires((line, number, position)),
            match opening {
                Some(active) => {
                    &&& active.start < active.content_start <= offset
                    &&& text.is_char_boundary(active.start)
                    &&& text.is_char_boundary(active.content_start)
                    &&& forall|i: int| 0 <= i < blocks.len() ==>
                        #[trigger] blocks@[i].span.end <= active.start
                },
                None => true,
            },
        decreases text.spec_bytes().len() - offset,
    {
        let end = next_line_end(text, offset);
        let line = span_text(text, offset, end);
        number += 1;
        let position = opening.as_ref().map(|active| active.body_position);
        let kind = match classifier(line, number, position) {
            Ok(kind) => kind,
            Err(error) => return Err(ScanFailure::Metadata(error)),
        };
        match kind {
            LineKind::Open { module, hash, mode } => {
                if opening.is_some() {
                    return Err(ScanFailure::NestedOpening { line: number });
                }
                opening = Some(Opening {
                    module, hash, mode, start: offset, content_start: end,
                    line: number, body_position: BodyPosition::First,
                });
            },
            LineKind::Close(module) => {
                let active = match opening {
                    Some(active) => active,
                    None => return Err(ScanFailure::MissingOpening { line: number }),
                };
                opening = None;
                if active.module != module {
                    return Err(ScanFailure::DifferentModule { line: number });
                }
                if module == wanted {
                    let content = span_text(text, active.content_start, offset);
                    blocks.push(ParsedBlock {
                        span: Span { start: active.start, end },
                        content, recorded_hash: active.hash, mode: active.mode,
                    });
                }
            },
            LineKind::Content | LineKind::LegacyHeader => {
                if let Some(active) = &mut opening {
                    if matches!(active.body_position, BodyPosition::First)
                        && matches!(kind, LineKind::LegacyHeader)
                    {
                        active.content_start = end;
                    }
                    active.body_position = BodyPosition::Later;
                }
            },
        }
        offset = end;
    }
    if let Some(active) = opening {
        return Err(ScanFailure::MissingClosing { line: active.line });
    }
    proof { scan_admits_splice(text.spec_bytes(), blocks@); }
    Ok(blocks)
}
}
