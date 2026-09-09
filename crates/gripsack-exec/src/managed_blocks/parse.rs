use std::ops::Range;

#[derive(Debug, thiserror::Error)]
#[error("malformed managed block at line {line}: {reason}")]
pub struct MergeParseError {
    pub line: usize,
    pub reason: &'static str,
}

pub(super) struct ParsedBlock<'a> {
    pub range: Range<usize>,
    pub content: &'a str,
    pub recorded_hash: &'a str,
    pub mode: Option<u32>,
}

struct Opening<'a> {
    module: &'a str,
    hash: &'a str,
    mode: Option<u32>,
    start: usize,
    content_start: usize,
    line: usize,
    first_body_line: bool,
}

enum Marker<'a> {
    Open {
        module: &'a str,
        hash: &'a str,
        mode: Option<u32>,
    },
    Close(&'a str),
}

fn marker(line: &str, number: usize) -> Result<Option<Marker<'_>>, MergeParseError> {
    let text = line.trim();
    let found = [
        (">>> gripsack module=", true),
        ("<<< gripsack module=", false),
    ]
    .into_iter()
    .find_map(|(key, open)| text.find(key).map(|index| (key, open, index)));
    let Some((key, open, index)) = found else {
        return Ok(None);
    };
    if text[..index].split_whitespace().count() > 1 {
        return Ok(None);
    }
    let fail = || MergeParseError {
        line: number,
        reason: "invalid marker metadata or delimiter",
    };
    let mut words = text[index + key.len()..].split_whitespace();
    let module = words
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(fail)?;
    if !open {
        if words.next() != Some("<<<") || !valid_tail(&mut words) {
            return Err(fail());
        }
        return Ok(Some(Marker::Close(module)));
    }
    let hash = words
        .next()
        .and_then(|word| word.strip_prefix("sha="))
        .filter(|hash| hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(fail)?;
    let mut end = words.next().ok_or_else(fail)?;
    let mode = if let Some(value) = end.strip_prefix("mode=") {
        let mode = value
            .starts_with('0')
            .then(|| u32::from_str_radix(value, 8).ok())
            .flatten()
            .filter(|mode| *mode <= 0o7777)
            .ok_or_else(fail)?;
        end = words.next().ok_or_else(fail)?;
        Some(mode)
    } else {
        None
    };
    if end != ">>>" || !valid_tail(&mut words) {
        return Err(fail());
    }
    Ok(Some(Marker::Open { module, hash, mode }))
}

fn valid_tail<'a>(words: &mut impl Iterator<Item = &'a str>) -> bool {
    match words.next() {
        None => true,
        Some("-->") => words.next().is_none(),
        _ => false,
    }
}

pub(super) fn validate_payload(payload: &str) -> Result<(), MergeParseError> {
    for (index, line) in payload.lines().enumerate() {
        if marker(line, index + 1)?.is_some() {
            return Err(MergeParseError {
                line: index + 1,
                reason: "payload contains a managed marker",
            });
        }
    }
    Ok(())
}

pub(super) fn scan<'a>(
    text: &'a str,
    wanted: &str,
) -> Result<Vec<ParsedBlock<'a>>, MergeParseError> {
    let mut blocks = Vec::new();
    let mut opening: Option<Opening<'a>> = None;
    let mut offset = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        let number = index + 1;
        match marker(line, number)? {
            Some(Marker::Open { module, hash, mode }) => {
                if opening.is_some() {
                    return Err(MergeParseError {
                        line: number,
                        reason: "nested or interleaved managed opener",
                    });
                }
                opening = Some(Opening {
                    module,
                    hash,
                    mode,
                    start: offset,
                    content_start: offset + line.len(),
                    line: number,
                    first_body_line: true,
                });
            }
            Some(Marker::Close(module)) => {
                let Some(active) = opening.take() else {
                    return Err(MergeParseError {
                        line: number,
                        reason: "closing marker has no opener",
                    });
                };
                if active.module != module {
                    return Err(MergeParseError {
                        line: number,
                        reason: "closing marker names a different module",
                    });
                }
                if module == wanted {
                    blocks.push(ParsedBlock {
                        range: active.start..offset + line.len(),
                        content: &text[active.content_start..offset],
                        recorded_hash: active.hash,
                        mode: active.mode,
                    });
                }
            }
            None => {
                if let Some(active) = &mut opening {
                    if active.first_body_line
                        && line
                            .contains("!! managed by gripsack — edit the module, not this block !!")
                    {
                        active.content_start = offset + line.len();
                    }
                    active.first_body_line = false;
                }
            }
        }
        offset += line.len();
    }
    if let Some(active) = opening {
        return Err(MergeParseError {
            line: active.line,
            reason: "opening marker has no closing marker; following text is unowned",
        });
    }
    Ok(blocks)
}
