//! The managed-block explorer (plan/0056 K5, F2 step 6).
//!
//! Preservation, idempotence and the content round trip are Lean theorems
//! (proofs/Kernel/Block.lean) for every output `block::upsert` returns; they
//! are re-checked here only as oracles of the explorer. What the explorer
//! adds is the property the theorems leave to testing: **liveness** — the
//! kernel never refuses a hosting text whose markers are well formed, for
//! every comment style the host uses and both owner spellings — and the
//! marker grammar itself (which lines are markers), which is not proved.
//!
//! Every hosting text of up to four lines over an alphabet of foreign lines,
//! markers of the wanted and another owner, the header, content and
//! malformed markers, with LF or CRLF and with or without a final newline,
//! is upserted with every payload and style. The calibration replays the
//! legacy content normalization (trim, then `\r\n` → `\n`) and requires the
//! round-trip oracle to catch it.
use gripsack_kernel::Digest;
use gripsack_kernel::block::{
    self, Block, BlockDecision, BlockIntent, BlockRecord, Host, LineKind, Newline, Seen,
    SpliceRefusal, Style,
};

const WANTED: &str = "m";
const WORKSPACE: &str = "w1-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A stand-in for the host's SHA-256: any injective-enough function works,
/// the kernel only records and compares it.
fn digest(bytes: &[u8]) -> Digest {
    let mut out = [0u8; 32];
    let mut state: u64 = 0xcbf2_9ce4_8422_2325;
    for (index, byte) in bytes.iter().enumerate() {
        state ^= u64::from(*byte);
        state = state.wrapping_mul(0x0100_0000_01b3);
        out[index % 32] ^= state as u8;
        out[(index + 7) % 32] = out[(index + 7) % 32].wrapping_add((state >> 32) as u8);
    }
    out[31] ^= bytes.len() as u8;
    Digest::from_bytes(out)
}

fn styles() -> Vec<Style> {
    vec![
        Style {
            prefix: b"#".to_vec(),
            html: false,
        },
        Style {
            prefix: b"<!--".to_vec(),
            html: true,
        },
        Style {
            prefix: b"--".to_vec(),
            html: false,
        },
        Style {
            prefix: "\u{10437}".as_bytes().to_vec(),
            html: false,
        },
    ]
}

const PAYLOADS: [&[u8]; 8] = [
    b"",
    b"a",
    b"export A=1\n",
    b"a\nb\n\n",
    b"a\r\r\nb\r\n",
    b"a\r\r\r\nb",
    b"x\ry\r",
    " \u{3000}\u{a0}λ\n\r\n".as_bytes(),
];

fn rendered_payload(payload: &[u8], style: &Style, owner: &[u8], newline: &Newline) -> Vec<u8> {
    let normalized = block::normalize(payload);
    block::render(
        style,
        owner,
        &block::mark(&digest(&normalized)),
        0o644,
        newline,
        &normalized,
    )
}

/// The line alphabet: rendered markers of both owners, their header and
/// content, foreign lines, and markers the grammar must refuse or ignore.
fn alphabet() -> Vec<Vec<u8>> {
    let style = Style {
        prefix: b"#".to_vec(),
        html: false,
    };
    let lines = |owner: &str| -> Vec<Vec<u8>> {
        let rendered = rendered_payload(b"a", &style, owner.as_bytes(), &Newline::Lf);
        rendered
            .split_inclusive(|byte| *byte == b'\n')
            .map(|line| line[..line.len() - 1].to_vec())
            .collect()
    };
    let wanted = lines(WANTED);
    let other = lines("o");
    vec![
        b"x".to_vec(),
        Vec::new(),
        wanted[0].clone(),                                 // open m
        wanted[1].clone(),                                 // header
        b"edited".to_vec(),                                // content
        wanted[3].clone(),                                 // close m
        other[0].clone(),                                  // open o
        other[3].clone(),                                  // close o
        b"a b # >>> gripsack module=m sha=0 >>>".to_vec(), // two words before: content
        b"# >>> gripsack module=m sha=xyz >>>".to_vec(),   // malformed metadata
    ]
}

fn texts() -> Vec<Vec<u8>> {
    let alphabet = alphabet();
    let mut out = Vec::new();
    let mut sequences: Vec<Vec<usize>> = vec![Vec::new()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for sequence in &sequences {
            for letter in 0..alphabet.len() {
                let mut longer = sequence.clone();
                longer.push(letter);
                next.push(longer);
            }
        }
        sequences.extend(next.iter().cloned());
        sequences.dedup();
        if sequences.len() > 20_000 {
            break;
        }
    }
    sequences.sort();
    sequences.dedup();
    for sequence in sequences {
        for crlf in [false, true] {
            for terminated in [true, false] {
                let mut text = Vec::new();
                for (index, letter) in sequence.iter().enumerate() {
                    text.extend_from_slice(&alphabet[*letter]);
                    if terminated || index + 1 < sequence.len() {
                        text.extend_from_slice(if crlf { b"\r\n" } else { b"\n" });
                    }
                }
                out.push(text);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The bytes outside the given spans, in order (an independent oracle).
fn outside(text: &[u8], blocks: &[Block]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cursor = 0;
    for span in blocks {
        out.extend_from_slice(&text[cursor..span.start]);
        cursor = span.end;
    }
    out.extend_from_slice(&text[cursor..]);
    out
}

fn has_marker_line(payload: &[u8]) -> bool {
    payload
        .split(|byte| *byte == b'\n')
        .any(|line| !matches!(block::classify(line, 0, line.len()), LineKind::Content))
}

#[test]
fn every_well_formed_text_upserts_idempotently_and_keeps_foreign_bytes() {
    let mut checked = 0usize;
    let mut appended = 0usize;
    let mut spliced = 0usize;
    for text in texts() {
        for owner in [WANTED.as_bytes(), WORKSPACE.as_bytes()] {
            let scanned = block::scan(&text, owner);
            for style in styles() {
                for payload in PAYLOADS {
                    let normalized = block::normalize(payload);
                    let id = digest(&normalized);
                    let result = block::upsert(&text, owner, &style, &id, 0o644, payload);
                    let blocks = match &scanned {
                        Err(_) => {
                            assert!(
                                matches!(result, Err(SpliceRefusal::Malformed(_))),
                                "a malformed text must refuse: {text:?}"
                            );
                            continue;
                        }
                        Ok(blocks) => blocks,
                    };
                    assert!(!has_marker_line(payload));
                    // liveness: a well-formed text is never refused
                    let out = match result {
                        Ok(out) => out,
                        Err(refusal) => panic!("refused {refusal:?}: {text:?} {payload:?}"),
                    };
                    let newline = block::newline_of(&text);
                    let rendered = block::render(
                        &style,
                        owner,
                        &block::mark(&id),
                        0o644,
                        &newline,
                        &normalized,
                    );
                    // preservation oracle
                    if blocks.is_empty() {
                        let mut expected = text.clone();
                        if text.last().is_some_and(|byte| *byte != b'\n') {
                            if matches!(newline, Newline::CrLf) {
                                expected.push(b'\r');
                            }
                            expected.push(b'\n');
                        }
                        let foreign = expected.clone();
                        expected.extend_from_slice(&rendered);
                        assert_eq!(out, expected);
                        assert_eq!(block::remove(&out, owner).ok().flatten(), Some(foreign));
                        appended += 1;
                    } else {
                        let foreign = outside(&text, blocks);
                        let mut expected = text[..blocks[0].start].to_vec();
                        expected.extend_from_slice(&rendered);
                        expected.extend_from_slice(&foreign[blocks[0].start..]);
                        assert_eq!(out, expected);
                        assert_eq!(block::remove(&out, owner).ok().flatten(), Some(foreign));
                        spliced += 1;
                    }
                    // round trip and idempotence
                    let again = block::scan(&out, owner).expect("output scans");
                    assert_eq!(again.len(), 1);
                    let found = &again[0];
                    assert_eq!(
                        block::normalize(&out[found.content_start..found.content_end]),
                        normalized
                    );
                    let mut sha = [0u8; block::MARK_DIGITS];
                    sha.copy_from_slice(&out[found.sha..found.sha + block::MARK_DIGITS]);
                    assert!(block::marks(&sha, &id));
                    assert_eq!(found.mode, Some(0o644));
                    let twice = block::upsert(&out, owner, &style, &id, 0o644, payload)
                        .expect("idempotent");
                    assert_eq!(twice, out);
                    checked += 1;
                }
            }
        }
    }
    assert!(appended > 1000 && spliced > 1000, "{appended} {spliced}");
    assert!(checked > 10_000, "{checked}");
}

#[test]
fn the_legacy_normalization_is_caught_by_the_round_trip_oracle() {
    // trim trailing \r/\n, then replace "\r\n" with "\n" (0.19.0 – 0.52.0)
    fn legacy(content: &[u8]) -> Vec<u8> {
        let mut end = content.len();
        while end > 0 && matches!(content[end - 1], b'\r' | b'\n') {
            end -= 1;
        }
        let text = String::from_utf8(content[..end].to_vec()).unwrap();
        text.replace("\r\n", "\n").into_bytes()
    }
    // the legacy renderer split with `str::lines`, which drops one '\r'
    // before each '\n'
    fn legacy_render(normalized: &[u8]) -> Vec<u8> {
        let text = String::from_utf8(normalized.to_vec()).unwrap();
        let mut body = Vec::new();
        for line in text.lines() {
            body.extend_from_slice(line.as_bytes());
            body.push(b'\n');
        }
        body
    }
    let round_trips = |normalizer: fn(&[u8]) -> Vec<u8>, render: fn(&[u8]) -> Vec<u8>| {
        PAYLOADS.iter().all(|payload| {
            let normal = normalizer(payload);
            normalizer(&render(&normal)) == normal
        })
    };
    let kernel_body = |normalized: &[u8]| -> Vec<u8> {
        let style = Style {
            prefix: b"#".to_vec(),
            html: false,
        };
        let rendered = block::render(&style, b"m", &[b'0'; 16], 0o644, &Newline::Lf, normalized);
        let found = &block::scan(&rendered, b"m").ok().unwrap()[0];
        rendered[found.content_start..found.content_end].to_vec()
    };
    assert!(PAYLOADS.iter().all(|payload| {
        let normal = block::normalize(payload);
        block::normalize(&kernel_body(&normal)) == normal
    }));
    assert!(
        !round_trips(legacy, legacy_render),
        "the oracle must catch the legacy rule"
    );
}

#[test]
fn rendering_is_the_marker_format_gripsack_has_always_written() {
    let id = digest(b"export A=1");
    let sha = block::mark(&id);
    let sha = std::str::from_utf8(&sha).unwrap();
    let out = block::upsert(
        b"# mine\r\n",
        WANTED.as_bytes(),
        &styles()[0],
        &id,
        0o600,
        b"export A=1\r\n",
    )
    .ok()
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        format!(
            "# mine\r\n# >>> gripsack module=m sha={sha} mode=0600 >>>\r\n\
             # !! managed by gripsack — edit the module, not this block !!\r\n\
             export A=1\r\n# <<< gripsack module=m <<<\r\n"
        )
    );
    let html = block::render(&styles()[1], b"m", &block::mark(&id), 0, &Newline::Lf, b"");
    assert_eq!(
        String::from_utf8(html).unwrap(),
        format!(
            "<!-- >>> gripsack module=m sha={sha} mode=00 >>> -->\n\
             <!-- !! managed by gripsack — edit the module, not this block !! -->\n\
             <!-- <<< gripsack module=m <<< -->\n"
        )
    );
}

#[test]
fn the_marker_grammar_is_unchanged() {
    let classify = |line: &str| block::classify(line.as_bytes(), 0, line.len());
    let open = "# >>> gripsack module=m sha=0123456789ABCDEF mode=0644 >>>";
    assert!(matches!(
        classify(open),
        LineKind::Open {
            mode: Some(0o644),
            ..
        }
    ));
    assert!(matches!(
        classify("  \u{3000}# >>> gripsack module=m sha=0123456789abcdef >>> --> \r\n"),
        LineKind::Open { mode: None, .. }
    ));
    assert!(matches!(
        classify("\u{10437} <<< gripsack module=m <<<"),
        LineKind::Close { .. }
    ));
    assert!(matches!(
        classify("# <<< gripsack module=m <<< x"),
        LineKind::Malformed
    ));
    assert!(matches!(
        classify("a b >>> gripsack module=m"),
        LineKind::Content
    ));
    assert!(matches!(
        classify("a\u{2003}b >>> gripsack module=m"),
        LineKind::Content
    ));
    for malformed in [
        "# >>> gripsack module=m sha=0123 >>>",
        "# >>> gripsack module=m sha=0123456789abcdef mode=644 >>>",
        "# >>> gripsack module=m sha=0123456789abcdef mode=010000 >>>",
        "# >>> gripsack module=m sha=0123456789abcdef mode=0644",
        "# >>> gripsack module=",
        "# <<< gripsack module=m",
    ] {
        assert!(
            matches!(classify(malformed), LineKind::Malformed),
            "{malformed}"
        );
    }
    assert!(matches!(
        classify("# >>> gripsack module=m sha=0123456789abcdef mode=00000000000000000017 >>>"),
        LineKind::Open {
            mode: Some(0o17),
            ..
        }
    ));
    // malformed structure never grants a span
    let text = |body: &str| body.replace('|', "\n").into_bytes();
    let first = "# >>> gripsack module=m sha=0123456789abcdef >>>";
    for malformed in [
        format!("{first}|USER-TAIL|"),
        format!("{first}|{first}|# <<< gripsack module=m <<<|"),
        "# <<< gripsack module=m <<<|".to_string(),
        format!("{first}|# <<< gripsack module=o <<<|"),
    ] {
        assert!(block::scan(&text(&malformed), b"m").is_err(), "{malformed}");
    }
    // another owner's well-formed block is walked, not returned
    let other =
        text("# >>> gripsack module=o sha=0123456789abcdef >>>|x|# <<< gripsack module=o <<<|");
    assert_eq!(block::scan(&other, b"m").ok().unwrap().len(), 0);
}

fn seen(content: u8, marked: bool, mode: Option<u32>) -> Seen {
    Seen {
        content: Digest::from_bytes([content; 32]),
        marked,
        mode,
    }
}

#[test]
fn the_block_decision_matches_the_planner_it_replaced() {
    let record = |hash: u8, mode: Option<u32>, preserved: bool| BlockRecord {
        hash: Some(Digest::from_bytes([hash; 32])),
        mode,
        preserved,
    };
    let deploy = BlockIntent::Deploy {
        desired: Digest::from_bytes([1; 32]),
        mode: 0o644,
    };
    let file = Host::File { mode: 0o644 };
    let decide = |r: Option<BlockRecord>, host: &Host, blocks: &[Seen], intent: &BlockIntent| {
        block::decide_block(&r, host, blocks, intent)
    };
    assert!(matches!(
        decide(None, &Host::Absent, &[], &deploy),
        BlockDecision::Write
    ));
    assert!(matches!(
        decide(None, &Host::Foreign, &[], &deploy),
        BlockDecision::Refuse(_)
    ));
    assert!(matches!(
        decide(None, &file, &[seen(1, true, Some(0o644))], &deploy),
        BlockDecision::Satisfied
    ));
    // a hand edit is regenerated (0.19.0), a duplicate reconciled (0.21.1)
    assert!(matches!(
        decide(None, &file, &[seen(1, false, Some(0o644))], &deploy),
        BlockDecision::Write
    ));
    assert!(matches!(
        decide(
            None,
            &file,
            &[seen(1, true, Some(0o644)), seen(1, true, None)],
            &deploy
        ),
        BlockDecision::Write
    ));
    // every mode record participates
    for blocks in [
        vec![seen(1, true, Some(0o600))],
        vec![seen(1, true, None), seen(1, true, Some(0o600))],
    ] {
        assert!(matches!(
            decide(None, &file, &blocks, &deploy),
            BlockDecision::Preserve
        ));
    }
    assert!(matches!(
        decide(
            Some(record(1, Some(0o600), false)),
            &file,
            &[seen(1, true, None)],
            &deploy
        ),
        BlockDecision::Preserve
    ));
    assert!(matches!(
        decide(
            Some(record(1, Some(0o600), true)),
            &file,
            &[seen(1, true, Some(0o644))],
            &deploy
        ),
        BlockDecision::Satisfied
    ));
    // removal only when intact
    let intact = Some(record(1, Some(0o644), false));
    assert!(matches!(
        decide(intact, &file, &[seen(1, true, None)], &BlockIntent::Remove),
        BlockDecision::Remove
    ));
    for (r, blocks) in [
        (record(1, Some(0o644), false), vec![seen(1, false, None)]),
        (record(2, Some(0o644), false), vec![seen(1, true, None)]),
        (record(1, Some(0o644), true), vec![seen(1, true, None)]),
        (record(1, Some(0o600), false), vec![seen(1, true, None)]),
        (record(1, None, false), vec![seen(1, true, None)]),
        (
            record(1, Some(0o644), false),
            vec![seen(1, true, None), seen(1, true, None)],
        ),
    ] {
        assert!(matches!(
            decide(Some(r), &file, &blocks, &BlockIntent::Remove),
            BlockDecision::Keep
        ));
    }
    assert!(matches!(
        decide(None, &Host::Foreign, &[], &BlockIntent::Remove),
        BlockDecision::Keep
    ));
    assert!(matches!(
        decide(None, &Host::Absent, &[], &BlockIntent::Remove),
        BlockDecision::Absent
    ));
    // rollback restores only a payload that is the record
    let restore = |retained: u8, recorded: u8| BlockIntent::Restore {
        retained: Digest::from_bytes([retained; 32]),
        recorded: Some(Digest::from_bytes([recorded; 32])),
        mode: 0o644,
    };
    assert!(matches!(
        decide(None, &file, &[], &restore(1, 1)),
        BlockDecision::Write
    ));
    assert!(matches!(
        decide(None, &file, &[], &restore(1, 2)),
        BlockDecision::Refuse(_)
    ));
    assert!(matches!(
        decide(None, &file, &[seen(1, true, Some(0o644))], &restore(1, 2)),
        BlockDecision::Refuse(_)
    ));
    assert!(matches!(
        decide(None, &file, &[seen(1, true, Some(0o600))], &restore(1, 2)),
        BlockDecision::Preserve
    ));
}
