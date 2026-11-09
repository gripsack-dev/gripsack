//! Managed blocks (plan/0056 §5 K5, F2 step 6): the marked span gripsack
//! owns inside a file other tools also write (`~/.bashrc`), and the decision
//! of when gripsack may touch it (K1 for blocks).
//!
//! Everything is stated over bytes. The kernel owns the whole round trip:
//! line classification (the marker grammar), the scan of a hosting text for
//! one owner's blocks, the content normalization the block's recorded hash
//! is taken over, rendering the marked block from its content, and the
//! splice that replaces or removes the owner's blocks.
//!
//! Proved (proofs/Kernel/Block.lean):
//!
//! - **Bytes outside the blocks are preserved exactly** (K5): an upsert's
//!   output is the text before the first block, the rendered block, then
//!   every byte between and after the owner's blocks, verbatim and in order;
//!   with no block, the text itself, one separating newline if it lacked
//!   one, then the block. A removal keeps exactly the bytes outside the
//!   blocks. The scanned blocks are ordered, disjoint and in bounds.
//! - **The splice is idempotent**: an upsert's output, upserted again with
//!   the same inputs, is returned unchanged. Its only block of this owner
//!   is the rendered block, whose content normalizes back to the payload's
//!   normalization and whose marker records the given digest and mode — so
//!   the next apply sees it intact, never "hand-edited".
//! - **K1 for blocks** ([`decide_block`]): a block is removed only if it is
//!   intact (exactly one block, its content is the record, its marker's sha
//!   matches its content, no mode drift, not preserved drift); a hosting
//!   object that is not a file is never written; conflicting mode evidence
//!   preserves the file; rollback restores a block only from a retained
//!   payload whose digest is the target's record.
//!
//! Idempotence is proved for every output the kernel returns. Upsert checks
//! its own output before returning it (it must scan back to exactly the
//! rendered block, with the same newline convention and content), and the
//! proof composes that check with the splice's specification. That upsert
//! never refuses a well-formed input is the property the kernel explorer
//! tests (`tests/block.rs`), not a theorem: a refusal is loud, never a
//! wrong write.
//!
//! Product semantics kept as they were (CHANGELOG 0.19.0, 0.21.1): inside
//! its markers a block is gripsack's, so apply regenerates a hand-edited
//! block and says so; a removal (prune, rollback's release) requires the
//! block intact and otherwise keeps it; malformed markers refuse.
//!
//! Aeneas shape (lib.rs): no derives, no iterators or closures, `while`
//! loops over explicit indices, owned facts, byte constants as arrays. The
//! clippy idioms allowed below (`RangeInclusive::contains`, `is_empty`,
//! `Option::map`, `is_some`) go through trait methods and closures the
//! translation does not model; the proofs are stated over this spelling.
#![allow(
    clippy::manual_range_contains,
    clippy::len_zero,
    clippy::manual_map,
    clippy::redundant_pattern_matching
)]
use crate::ids::Digest;
use alloc::vec::Vec;

/// `>>> gripsack module=`: the open marker's key.
const OPEN_KEY: [u8; 20] = *b">>> gripsack module=";
/// `<<< gripsack module=`: the close marker's key.
const CLOSE_KEY: [u8; 20] = *b"<<< gripsack module=";
/// The header line written after the open marker. Pre-hash blocks carried it
/// as the first content line; either way it is not content.
const HEADER: [u8; 61] = *b"!! managed by gripsack \xe2\x80\x94 edit the module, not this block !!";
const SHA_KEY: [u8; 4] = *b"sha=";
const MODE_KEY: [u8; 5] = *b"mode=";
/// The open and close markers end in a word of three `>` / `<` (a repeated
/// byte array constant is one Aeneas cannot translate, so they are written
/// and checked as a byte thrice).
const OPEN_END: u8 = b'>';
const CLOSE_END: u8 = b'<';
const HTML_END: [u8; 3] = *b"-->";
/// The marker records the first 16 hex digits of the content digest.
pub const MARK_DIGITS: usize = 16;
/// The largest mode a marker records (`0o7777`).
pub const MODE_MAX: u32 = 4095;

// ---- the marker grammar ----------------------------------------------------

/// The byte length of the Unicode `White_Space` character at `i` (which is
/// before `to`), or 0: exactly `char::is_whitespace`, which the marker
/// grammar has always used through `str::trim` and `split_whitespace`.
fn whitespace_at(text: &[u8], i: usize, to: usize) -> usize {
    let lead = text[i];
    if lead == b' ' || (lead >= 9 && lead <= 13) {
        return 1;
    }
    if lead == 0xc2 && i + 1 < to {
        let next = text[i + 1];
        if next == 0x85 || next == 0xa0 {
            return 2;
        }
        return 0;
    }
    if (lead == 0xe1 || lead == 0xe2 || lead == 0xe3) && i + 2 < to {
        let second = text[i + 1];
        let third = text[i + 2];
        let wide = if lead == 0xe1 {
            second == 0x9a && third == 0x80
        } else if lead == 0xe3 {
            second == 0x80 && third == 0x80
        } else if second == 0x80 {
            (third >= 0x80 && third <= 0x8a) || third == 0xa8 || third == 0xa9 || third == 0xaf
        } else {
            second == 0x81 && third == 0x9f
        };
        if wide {
            return 3;
        }
    }
    0
}

/// The next whitespace-separated word in `text[from..to]`, as `(start,
/// end)`; `(to, to)` when there is none.
fn next_word(text: &[u8], from: usize, to: usize) -> (usize, usize) {
    let mut i = from;
    while i < to {
        let width = whitespace_at(text, i, to);
        if width == 0 {
            break;
        }
        i += width;
    }
    if i >= to {
        return (to, to);
    }
    let start = i;
    while i < to {
        if whitespace_at(text, i, to) != 0 {
            break;
        }
        i += 1;
    }
    (start, i)
}

/// `str::trim` as a range: the first to the last non-whitespace byte.
fn trimmed(text: &[u8], from: usize, to: usize) -> (usize, usize) {
    let mut i = from;
    let mut start = to;
    let mut end = to;
    let mut seen = false;
    while i < to {
        let width = whitespace_at(text, i, to);
        if width != 0 {
            i += width;
        } else {
            if !seen {
                start = i;
                seen = true;
            }
            i += 1;
            end = i;
        }
    }
    (start, end)
}

/// Whether `text[at..]` starts with `key` (`at + key.len() <= text.len()`).
fn matches_at(text: &[u8], at: usize, key: &[u8]) -> bool {
    let mut j = 0;
    while j < key.len() {
        if text[at + j] != key[j] {
            return false;
        }
        j += 1;
    }
    true
}

/// `str::find`: the first position in `text[from..to]` where `key` starts.
fn find(text: &[u8], from: usize, to: usize, key: &[u8]) -> Option<usize> {
    if to < from || to - from < key.len() {
        return None;
    }
    let last = to - key.len();
    let mut i = from;
    while i <= last {
        if matches_at(text, i, key) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Whether `text[start..end]` is exactly three `byte`s.
fn is_triple(text: &[u8], start: usize, end: usize, byte: u8) -> bool {
    end - start == 3 && text[start] == byte && text[start + 1] == byte && text[start + 2] == byte
}

fn push_triple(out: &mut Vec<u8>, byte: u8) {
    out.push(byte);
    out.push(byte);
    out.push(byte);
}

/// Whether `text[start..end]` is exactly `word`.
fn is_word(text: &[u8], start: usize, end: usize, word: &[u8]) -> bool {
    end - start == word.len() && matches_at(text, start, word)
}

/// Whether two ranges of `text` hold the same bytes.
fn same_range(text: &[u8], a: usize, a_end: usize, b: usize, b_end: usize) -> bool {
    if a_end - a != b_end - b {
        return false;
    }
    let mut j = 0;
    while j < a_end - a {
        if text[a + j] != text[b + j] {
            return false;
        }
        j += 1;
    }
    true
}

/// At most one word in `text[from..to]`.
fn at_most_one_word(text: &[u8], from: usize, to: usize) -> bool {
    let (_, end) = next_word(text, from, to);
    let (start, _) = next_word(text, end, to);
    start == to
}

fn hex_digit(byte: u8) -> bool {
    (byte >= b'0' && byte <= b'9')
        || (byte >= b'a' && byte <= b'f')
        || (byte >= b'A' && byte <= b'F')
}

/// `sha=` followed by exactly 16 hex digits; the digits' offset.
fn sha_word(text: &[u8], start: usize, end: usize) -> Option<usize> {
    if end - start != SHA_KEY.len() + MARK_DIGITS || !matches_at(text, start, &SHA_KEY) {
        return None;
    }
    let mut i = start + SHA_KEY.len();
    while i < end {
        if !hex_digit(text[i]) {
            return None;
        }
        i += 1;
    }
    Some(start + SHA_KEY.len())
}

/// A leading-zero octal mode no larger than `0o7777` (the grammar has always
/// been `u32::from_str_radix(value, 8)` of a value starting with `0`).
fn octal_mode(text: &[u8], start: usize, end: usize) -> Option<u32> {
    if start >= end || text[start] != b'0' {
        return None;
    }
    let mut value: u32 = 0;
    let mut i = start;
    while i < end {
        let digit = text[i];
        if digit < b'0' || digit > b'7' {
            return None;
        }
        value = value * 8 + (digit - b'0') as u32;
        if value > MODE_MAX {
            return None;
        }
        i += 1;
    }
    Some(value)
}

/// Nothing after the closing delimiter but, optionally, an HTML `-->`.
fn valid_tail(text: &[u8], from: usize, to: usize) -> bool {
    let (start, end) = next_word(text, from, to);
    if start == to {
        return true;
    }
    if !is_word(text, start, end, &HTML_END) {
        return false;
    }
    let (rest, _) = next_word(text, end, to);
    rest == to
}

/// One line's role in the marker grammar. Offsets index the whole text.
pub enum LineKind {
    Content,
    Open {
        owner_start: usize,
        owner_end: usize,
        sha: usize,
        mode: Option<u32>,
    },
    Close {
        owner_start: usize,
        owner_end: usize,
    },
    /// A marker key with metadata the grammar refuses.
    Malformed,
}

/// Classify the line `text[from..to]`: a marker is its key, preceded by at
/// most one word (the comment prefix), followed by the owner and metadata.
pub fn classify(text: &[u8], from: usize, to: usize) -> LineKind {
    let (start, end) = trimmed(text, from, to);
    let (at, open) = match find(text, start, end, &OPEN_KEY) {
        Some(at) => (at, true),
        None => match find(text, start, end, &CLOSE_KEY) {
            Some(at) => (at, false),
            None => return LineKind::Content,
        },
    };
    if !at_most_one_word(text, start, at) {
        return LineKind::Content;
    }
    let (owner_start, owner_end) = next_word(text, at + OPEN_KEY.len(), end);
    if owner_start == end {
        return LineKind::Malformed;
    }
    if !open {
        let (word, word_end) = next_word(text, owner_end, end);
        if !is_triple(text, word, word_end, CLOSE_END) || !valid_tail(text, word_end, end) {
            return LineKind::Malformed;
        }
        return LineKind::Close {
            owner_start,
            owner_end,
        };
    }
    let (hash, hash_end) = next_word(text, owner_end, end);
    let sha = match sha_word(text, hash, hash_end) {
        Some(sha) => sha,
        None => return LineKind::Malformed,
    };
    let (mut word, mut word_end) = next_word(text, hash_end, end);
    if word == end {
        return LineKind::Malformed;
    }
    let mut mode = None;
    if word_end - word >= MODE_KEY.len() && matches_at(text, word, &MODE_KEY) {
        match octal_mode(text, word + MODE_KEY.len(), word_end) {
            Some(value) => mode = Some(value),
            None => return LineKind::Malformed,
        }
        let (next, next_end) = next_word(text, word_end, end);
        if next == end {
            return LineKind::Malformed;
        }
        word = next;
        word_end = next_end;
    }
    if !is_triple(text, word, word_end, OPEN_END) || !valid_tail(text, word_end, end) {
        return LineKind::Malformed;
    }
    LineKind::Open {
        owner_start,
        owner_end,
        sha,
        mode,
    }
}

/// Whether `text[from..to]` contains `key`.
fn contains(text: &[u8], from: usize, to: usize, key: &[u8]) -> bool {
    match find(text, from, to, key) {
        Some(_) => true,
        None => false,
    }
}

// ---- the scan -------------------------------------------------------------

/// The end of the line starting at `start` (after its `\n`, or the text's
/// end).
fn line_end(text: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < text.len() {
        if text[i] == b'\n' {
            return i + 1;
        }
        i += 1;
    }
    text.len()
}

/// One block of the wanted owner, as offsets into the scanned text: the
/// marked span `start..end` (open marker line through close marker line),
/// its content `content_start..content_end` (between the header and the
/// close marker), the marker's 16 recorded sha digits at `sha`, and its
/// recorded mode.
pub struct Block {
    pub start: usize,
    pub content_start: usize,
    pub content_end: usize,
    pub end: usize,
    pub sha: usize,
    pub mode: Option<u32>,
}

/// Why a hosting text has no trustworthy block structure. Every failure
/// refuses: no span is granted past a marker the grammar cannot place.
pub enum Malformation {
    /// A marker key with metadata the grammar refuses.
    Metadata,
    /// An open marker inside an open block.
    Nested,
    /// A close marker with no open marker.
    Orphan,
    /// A close marker naming another owner than its open marker.
    Interleaved,
    /// An open marker never closed: the following text is not owned.
    Unclosed,
}

/// A malformation and the (1-based) line it is reported at.
pub struct ScanFailure {
    pub line: usize,
    pub malformation: Malformation,
}

/// An open block while scanning.
struct Opening {
    start: usize,
    content_start: usize,
    line: usize,
    owner_start: usize,
    owner_end: usize,
    sha: usize,
    mode: Option<u32>,
    /// No body line seen yet (the header may follow).
    first: bool,
}

/// Every block of `owner` in `text`, in order. Blocks of other owners are
/// walked (their markers must nest correctly too) but not returned.
pub fn scan(text: &[u8], owner: &[u8]) -> Result<Vec<Block>, ScanFailure> {
    let mut blocks = Vec::new();
    let mut opening: Option<Opening> = None;
    let mut offset = 0;
    let mut number = 0;
    while offset < text.len() {
        let end = line_end(text, offset);
        number += 1;
        match classify(text, offset, end) {
            LineKind::Malformed => {
                return Err(ScanFailure {
                    line: number,
                    malformation: Malformation::Metadata,
                });
            }
            LineKind::Open {
                owner_start,
                owner_end,
                sha,
                mode,
            } => {
                if opening.is_some() {
                    return Err(ScanFailure {
                        line: number,
                        malformation: Malformation::Nested,
                    });
                }
                opening = Some(Opening {
                    start: offset,
                    content_start: end,
                    line: number,
                    owner_start,
                    owner_end,
                    sha,
                    mode,
                    first: true,
                });
            }
            LineKind::Close {
                owner_start,
                owner_end,
            } => {
                let active = match opening {
                    Some(active) => active,
                    None => {
                        return Err(ScanFailure {
                            line: number,
                            malformation: Malformation::Orphan,
                        });
                    }
                };
                opening = None;
                if !same_range(
                    text,
                    active.owner_start,
                    active.owner_end,
                    owner_start,
                    owner_end,
                ) {
                    return Err(ScanFailure {
                        line: number,
                        malformation: Malformation::Interleaved,
                    });
                }
                if owner_end - owner_start == owner.len() && matches_at(text, owner_start, owner) {
                    blocks.push(Block {
                        start: active.start,
                        content_start: active.content_start,
                        content_end: offset,
                        end,
                        sha: active.sha,
                        mode: active.mode,
                    });
                }
            }
            LineKind::Content => {
                opening = match opening {
                    Some(active) => Some(Opening {
                        start: active.start,
                        content_start: if active.first && contains(text, offset, end, &HEADER) {
                            end
                        } else {
                            active.content_start
                        },
                        line: active.line,
                        owner_start: active.owner_start,
                        owner_end: active.owner_end,
                        sha: active.sha,
                        mode: active.mode,
                        first: false,
                    }),
                    None => None,
                };
            }
        }
        offset = end;
    }
    match opening {
        Some(active) => Err(ScanFailure {
            line: active.line,
            malformation: Malformation::Unclosed,
        }),
        None => Ok(blocks),
    }
}

// ---- content and rendering -------------------------------------------------

/// The content a block's hash is taken over: every line's trailing carriage
/// returns dropped, then trailing carriage returns and newlines trimmed.
///
/// This is the old rule (trim, then `\r\n` → `\n`) wherever that rule was
/// stable. Where it was not — a carriage-return run before a newline — the
/// old normal form was not the normal form of the block rendered from it, so
/// the block never matched its own sha. This rule is a fixed point of
/// render-then-normalize.
pub fn normalize(content: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    // carriage returns not yet known to precede something other than '\n'
    let mut returns = 0;
    // the length of `out` up to its last byte that is not '\n'
    let mut keep = 0;
    let mut i = 0;
    while i < content.len() {
        let byte = content[i];
        if byte == b'\r' {
            returns += 1;
        } else if byte == b'\n' {
            returns = 0;
            out.push(b'\n');
        } else {
            while returns > 0 {
                out.push(b'\r');
                returns -= 1;
            }
            out.push(byte);
            keep = out.len();
        }
        i += 1;
    }
    let mut normal: Vec<u8> = Vec::new();
    normal.extend_from_slice(&out[..keep]);
    normal
}

/// The newline convention of a hosting text, which a rendered block follows.
pub enum Newline {
    Lf,
    CrLf,
}

impl Newline {
    pub fn same(&self, other: &Newline) -> bool {
        match (self, other) {
            (Newline::Lf, Newline::Lf) => true,
            (Newline::CrLf, Newline::CrLf) => true,
            (Newline::Lf, Newline::CrLf) => false,
            (Newline::CrLf, Newline::Lf) => false,
        }
    }
}

/// CRLF if the text contains a `\r\n` anywhere, else LF.
pub fn newline_of(text: &[u8]) -> Newline {
    let mut i = 0;
    while i + 1 < text.len() {
        if text[i] == b'\r' && text[i + 1] == b'\n' {
            return Newline::CrLf;
        }
        i += 1;
    }
    Newline::Lf
}

fn push_newline(out: &mut Vec<u8>, newline: &Newline) {
    match newline {
        Newline::Lf => {}
        Newline::CrLf => out.push(b'\r'),
    }
    out.push(b'\n');
}

/// How the hosting file comments: a line prefix and, for HTML-like files, a
/// ` -->` closing each marker line.
pub struct Style {
    pub prefix: Vec<u8>,
    pub html: bool,
}

fn push_style_end(out: &mut Vec<u8>, style: &Style, newline: &Newline) {
    if style.html {
        out.push(b' ');
        out.extend_from_slice(&HTML_END);
    }
    push_newline(out, newline);
}

/// The 16 lowercase hex digits a marker records for `digest`.
pub fn mark(digest: &Digest) -> [u8; MARK_DIGITS] {
    let bytes = digest.as_bytes();
    let mut digits = [0u8; MARK_DIGITS];
    let mut i = 0;
    while i < MARK_DIGITS / 2 {
        digits[2 * i] = hex_lower(bytes[i] / 16);
        digits[2 * i + 1] = hex_lower(bytes[i] % 16);
        i += 1;
    }
    digits
}

fn hex_lower(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + (nibble - 10)
    }
}

fn lower(byte: u8) -> u8 {
    if byte >= b'A' && byte <= b'Z' {
        byte + 32
    } else {
        byte
    }
}

/// Whether a marker's recorded sha digits name `digest` (case-insensitive,
/// as recorded markers always were compared).
pub fn marks(sha: &[u8; MARK_DIGITS], digest: &Digest) -> bool {
    let expected = mark(digest);
    let mut i = 0;
    while i < MARK_DIGITS {
        if lower(sha[i]) != expected[i] {
            return false;
        }
        i += 1;
    }
    true
}

fn push_octal(out: &mut Vec<u8>, mode: u32) {
    let mut digits: Vec<u8> = Vec::new();
    let mut rest = mode;
    digits.push(b'0' + (rest % 8) as u8);
    rest /= 8;
    while rest > 0 {
        digits.push(b'0' + (rest % 8) as u8);
        rest /= 8;
    }
    let mut i = digits.len();
    while i > 0 {
        i -= 1;
        out.push(digits[i]);
    }
}

/// The marked block for `owner` holding `normalized` content: the open
/// marker (owner, recorded sha digits, mode), the header, one line per
/// content line, the close marker — every line ending in `newline`.
pub fn render(
    style: &Style,
    owner: &[u8],
    sha: &[u8; MARK_DIGITS],
    mode: u32,
    newline: &Newline,
    normalized: &[u8],
) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(&style.prefix);
    out.push(b' ');
    out.extend_from_slice(&OPEN_KEY);
    out.extend_from_slice(owner);
    out.push(b' ');
    out.extend_from_slice(&SHA_KEY);
    out.extend_from_slice(sha);
    out.push(b' ');
    out.extend_from_slice(&MODE_KEY);
    out.push(b'0');
    push_octal(&mut out, mode);
    out.push(b' ');
    push_triple(&mut out, OPEN_END);
    push_style_end(&mut out, style, newline);
    out.extend_from_slice(&style.prefix);
    out.push(b' ');
    out.extend_from_slice(&HEADER);
    push_style_end(&mut out, style, newline);
    let mut start = 0;
    let mut i = 0;
    while i < normalized.len() {
        if normalized[i] == b'\n' {
            out.extend_from_slice(&normalized[start..i]);
            push_newline(&mut out, newline);
            start = i + 1;
        }
        i += 1;
    }
    if start < normalized.len() {
        out.extend_from_slice(&normalized[start..]);
        push_newline(&mut out, newline);
    }
    out.extend_from_slice(&style.prefix);
    out.push(b' ');
    out.extend_from_slice(&CLOSE_KEY);
    out.extend_from_slice(owner);
    out.push(b' ');
    push_triple(&mut out, CLOSE_END);
    push_style_end(&mut out, style, newline);
    out
}

// ---- the splice ------------------------------------------------------------

/// Whether the blocks are in order, disjoint and inside a text of `len`
/// bytes, each block's marked span non-empty.
pub fn ordered(blocks: &[Block], len: usize) -> bool {
    let mut cursor = 0;
    let mut i = 0;
    while i < blocks.len() {
        let span = &blocks[i];
        if span.start < cursor || span.end <= span.start || span.end > len {
            return false;
        }
        cursor = span.end;
        i += 1;
    }
    true
}

/// Replace the first block's marked span with `replacement` and drop every
/// other block's span; every byte outside the spans is copied verbatim.
/// Requires [`ordered`] blocks.
pub fn splice(text: &[u8], blocks: &[Block], replacement: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut cursor = 0;
    let mut i = 0;
    while i < blocks.len() {
        out.extend_from_slice(&text[cursor..blocks[i].start]);
        if i == 0 {
            out.extend_from_slice(replacement);
        }
        cursor = blocks[i].end;
        i += 1;
    }
    out.extend_from_slice(&text[cursor..]);
    out
}

/// Append `block` to a text with no block of its owner: the text, a newline
/// if it is non-empty and does not end in one, then the block.
pub fn append(text: &[u8], newline: &Newline, rendered: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(text);
    if text.len() > 0 && text[text.len() - 1] != b'\n' {
        push_newline(&mut out, newline);
    }
    out.extend_from_slice(rendered);
    out
}

/// Why the kernel did not splice.
pub enum SpliceRefusal {
    /// The hosting text's markers cannot be placed.
    Malformed(ScanFailure),
    /// The scan returned blocks that are not ordered (unreachable for this
    /// scanner; checked so the splice's precondition is never assumed).
    Unordered,
    /// The rendered block does not read back as written: an owner, comment
    /// prefix or payload line the marker grammar would misread.
    Unrecognized,
}

/// Whether `bytes` equals `text[start..end]`.
fn holds(text: &[u8], start: usize, end: usize, bytes: &[u8]) -> bool {
    end <= text.len()
        && start <= end
        && end - start == bytes.len()
        && matches_at(text, start, bytes)
}

/// The postcondition upsert checks before returning its output: the output
/// holds exactly one block of `owner`, which is the rendered block, keeps
/// the newline convention the block was rendered with, its marker records
/// `sha` and `mode`, and its content normalizes to the payload's
/// normalization.
fn settled(
    out: &[u8],
    owner: &[u8],
    rendered: &[u8],
    normalized: &[u8],
    newline: &Newline,
    sha: &[u8; MARK_DIGITS],
    mode: u32,
) -> bool {
    let blocks = match scan(out, owner) {
        Ok(blocks) => blocks,
        Err(_) => return false,
    };
    if blocks.len() != 1 || !ordered(&blocks, out.len()) {
        return false;
    }
    let found = &blocks[0];
    if !holds(out, found.start, found.end, rendered) || !newline_of(out).same(newline) {
        return false;
    }
    if !holds(out, found.sha, found.sha + MARK_DIGITS, sha) || !same_mode(&found.mode, mode) {
        return false;
    }
    if found.content_start > found.content_end || found.content_end > out.len() {
        return false;
    }
    let content = normalize(&out[found.content_start..found.content_end]);
    content.len() == normalized.len() && matches_at(&content, 0, normalized)
}

/// Deploy `payload` as `owner`'s block in `text`, recording `digest` (the
/// host's SHA-256 of [`normalize`]`(payload)`) and `mode`. Replaces the
/// first block of `owner` and drops any duplicates; appends when there is
/// none. Refuses malformed markers, and any output that does not read back
/// as exactly the rendered block.
pub fn upsert(
    text: &[u8],
    owner: &[u8],
    style: &Style,
    digest: &Digest,
    mode: u32,
    payload: &[u8],
) -> Result<Vec<u8>, SpliceRefusal> {
    let blocks = match scan(text, owner) {
        Ok(blocks) => blocks,
        Err(failure) => return Err(SpliceRefusal::Malformed(failure)),
    };
    if !ordered(&blocks, text.len()) {
        return Err(SpliceRefusal::Unordered);
    }
    let newline = newline_of(text);
    let normalized = normalize(payload);
    let sha = mark(digest);
    let rendered = render(style, owner, &sha, mode, &newline, &normalized);
    let out = if blocks.len() == 0 {
        append(text, &newline, &rendered)
    } else {
        splice(text, &blocks, &rendered)
    };
    if settled(&out, owner, &rendered, &normalized, &newline, &sha, mode) {
        Ok(out)
    } else {
        Err(SpliceRefusal::Unrecognized)
    }
}

/// Remove every block of `owner` from `text`, keeping every other byte;
/// `None` when there is no block to remove.
pub fn remove(text: &[u8], owner: &[u8]) -> Result<Option<Vec<u8>>, SpliceRefusal> {
    let blocks = match scan(text, owner) {
        Ok(blocks) => blocks,
        Err(failure) => return Err(SpliceRefusal::Malformed(failure)),
    };
    if !ordered(&blocks, text.len()) {
        return Err(SpliceRefusal::Unordered);
    }
    if blocks.len() == 0 {
        return Ok(None);
    }
    Ok(Some(splice(text, &blocks, &[])))
}

// ---- the decision (K1 for blocks) -----------------------------------------

/// The object at the hosting path.
pub enum Host {
    Absent,
    /// A regular file holding text, with its permission mode.
    File {
        mode: u32,
    },
    /// A link, a directory or a special file: never gripsack's to splice.
    Foreign,
}

/// One scanned block of the owner, as the decision sees it.
pub struct Seen {
    /// The digest of the block's normalized content.
    pub content: Digest,
    /// Whether the marker's recorded sha digits name `content` (false means
    /// the block was edited by hand).
    pub marked: bool,
    /// The mode the marker records, if any.
    pub mode: Option<u32>,
}

/// What a generation's manifest records for the block.
pub struct BlockRecord {
    /// The recorded content digest (`None`: a record that is not one).
    pub hash: Option<Digest>,
    /// The recorded hosting-file mode, if any.
    pub mode: Option<u32>,
    /// The record is preserved drift: an observation, never authority.
    pub preserved: bool,
}

/// What this run wants for the block.
pub enum BlockIntent {
    /// Apply: the block holds the payload whose normalized digest is
    /// `desired`, in a hosting file of `mode`.
    Deploy { desired: Digest, mode: u32 },
    /// Rollback: restore the target generation's record from the retained
    /// payload whose normalized digest is `retained`.
    Restore {
        retained: Digest,
        recorded: Option<Digest>,
        mode: u32,
    },
    /// Prune or rollback's release: take gripsack's block out.
    Remove,
}

/// Why a block is left alone.
pub enum BlockRefusal {
    /// The hosting path is not a file gripsack can splice.
    Foreign,
    /// No retained payload reproduces the record (rollback).
    Unrestorable,
}

/// The per-block decision.
pub enum BlockDecision {
    /// Splice the rendered block in (replacing any hand edit inside it).
    Write,
    /// The block is already the intent in a file of the intended mode.
    Satisfied,
    /// Mode evidence conflicts with the live file: keep the whole file.
    Preserve,
    /// Remove gripsack's intact block.
    Remove,
    /// The block changed since gripsack wrote it: keep it.
    Keep,
    /// Nothing to remove.
    Absent,
    Refuse(BlockRefusal),
}

impl BlockDecision {
    /// Whether this decision changes the hosting file.
    pub fn writes(&self) -> bool {
        match self {
            BlockDecision::Write => true,
            BlockDecision::Remove => true,
            BlockDecision::Satisfied => false,
            BlockDecision::Preserve => false,
            BlockDecision::Keep => false,
            BlockDecision::Absent => false,
            BlockDecision::Refuse(_) => false,
        }
    }
}

fn same_digest(a: &Option<Digest>, b: &Digest) -> bool {
    match a {
        Some(a) => a.same(b),
        None => false,
    }
}

fn same_mode(a: &Option<u32>, mode: u32) -> bool {
    match a {
        Some(a) => *a == mode,
        None => false,
    }
}

/// A recorded mode other than `live`.
fn other_mode(mode: &Option<u32>, live: u32) -> bool {
    match mode {
        Some(mode) => *mode != live,
        None => false,
    }
}

/// Some block, or the (non-drift) record, names a mode other than `live`.
fn mode_conflict(record: &Option<BlockRecord>, blocks: &[Seen], live: u32) -> bool {
    let mut i = 0;
    while i < blocks.len() {
        if other_mode(&blocks[i].mode, live) {
            return true;
        }
        i += 1;
    }
    match record {
        Some(record) => {
            if record.preserved {
                false
            } else {
                match record.mode {
                    Some(mode) => mode != live,
                    None => false,
                }
            }
        }
        None => false,
    }
}

/// Exactly one block, unedited, holding `desired`, marked with `mode`.
fn holds_intent(blocks: &[Seen], desired: &Digest, mode: u32) -> bool {
    if blocks.len() != 1 {
        return false;
    }
    let only = &blocks[0];
    only.marked && only.content.same(desired) && same_mode(&only.mode, mode)
}

/// The block is exactly what the record says gripsack wrote: one block,
/// unedited, holding the recorded content, the record and marker agreeing
/// with the live mode, and the record not preserved drift.
pub fn intact(record: &BlockRecord, blocks: &[Seen], live: u32) -> bool {
    if record.preserved || blocks.len() != 1 {
        return false;
    }
    let only = &blocks[0];
    let recorded_mode = match record.mode {
        Some(mode) => Some(mode),
        None => only.mode,
    };
    let marker_agrees = match only.mode {
        Some(mode) => mode == live,
        None => true,
    };
    same_mode(&recorded_mode, live)
        && marker_agrees
        && same_digest(&record.hash, &only.content)
        && only.marked
}

/// Deploy or restore `desired` (rollback's retained payload is already
/// checked against its record).
fn deploy(
    record: &Option<BlockRecord>,
    host: &Host,
    blocks: &[Seen],
    desired: &Digest,
    mode: u32,
) -> BlockDecision {
    match host {
        Host::Foreign => BlockDecision::Refuse(BlockRefusal::Foreign),
        Host::Absent => BlockDecision::Write,
        Host::File { mode: live } => {
            if blocks.len() > 0 && mode_conflict(record, blocks, *live) {
                BlockDecision::Preserve
            } else if *live == mode && holds_intent(blocks, desired, mode) {
                BlockDecision::Satisfied
            } else {
                BlockDecision::Write
            }
        }
    }
}

/// Decide one managed block. `record` is the record gripsack acts from: the
/// previous generation's for a deploy, the current generation's for a
/// restore or a removal. `blocks` are the owner's blocks scanned from the
/// live hosting file (empty when it is absent).
pub fn decide_block(
    record: &Option<BlockRecord>,
    host: &Host,
    blocks: &[Seen],
    intent: &BlockIntent,
) -> BlockDecision {
    match intent {
        BlockIntent::Deploy { desired, mode } => deploy(record, host, blocks, desired, *mode),
        BlockIntent::Restore {
            retained,
            recorded,
            mode,
        } => {
            let decision = deploy(record, host, blocks, retained, *mode);
            if same_digest(recorded, retained) {
                decision
            } else {
                // the retained payload is not the record: writing it would
                // restore something else, and finding it already in place
                // would claim a restore that did not happen; drift and a
                // foreign host are left alone as for any restore
                match decision {
                    BlockDecision::Write => BlockDecision::Refuse(BlockRefusal::Unrestorable),
                    BlockDecision::Satisfied => BlockDecision::Refuse(BlockRefusal::Unrestorable),
                    other => other,
                }
            }
        }
        BlockIntent::Remove => match host {
            Host::Absent => BlockDecision::Absent,
            Host::Foreign => BlockDecision::Keep,
            Host::File { mode: live } => {
                if blocks.len() == 0 {
                    BlockDecision::Absent
                } else {
                    match record {
                        Some(record) => {
                            if intact(record, blocks, *live) {
                                BlockDecision::Remove
                            } else {
                                BlockDecision::Keep
                            }
                        }
                        None => BlockDecision::Keep,
                    }
                }
            }
        },
    }
}
