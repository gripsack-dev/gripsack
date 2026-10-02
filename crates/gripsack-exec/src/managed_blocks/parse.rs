use gripsack_policy::merge::scanner;

#[derive(Debug, thiserror::Error)]
#[error("malformed managed block at line {line}: {reason}")]
pub struct MergeParseError {
    pub line: usize,
    pub reason: &'static str,
}

fn marker(line: &str, number: usize) -> Result<scanner::LineKind<'_>, MergeParseError> {
    let text = line.trim();
    let found = [
        (">>> gripsack module=", true),
        ("<<< gripsack module=", false),
    ]
    .into_iter()
    .find_map(|(key, open)| text.find(key).map(|index| (key, open, index)));
    let Some((key, open, index)) = found else {
        return Ok(scanner::LineKind::Content);
    };
    if text[..index].split_whitespace().count() > 1 {
        return Ok(scanner::LineKind::Content);
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
        return Ok(scanner::LineKind::Close(module));
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
    Ok(scanner::LineKind::Open { module, hash, mode })
}

fn valid_tail<'a>(words: &mut impl Iterator<Item = &'a str>) -> bool {
    match words.next() {
        None => true,
        Some("-->") => words.next().is_none(),
        _ => false,
    }
}

pub(crate) fn validate_payload(payload: &str) -> Result<(), MergeParseError> {
    for (index, line) in payload.lines().enumerate() {
        if !matches!(marker(line, index + 1)?, scanner::LineKind::Content) {
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
) -> Result<Vec<scanner::ParsedBlock<'a>>, MergeParseError> {
    scanner::scan(text, wanted, |line, number, position| {
        let kind = marker(line, number)?;
        if matches!(kind, scanner::LineKind::Content)
            && matches!(position, Some(scanner::BodyPosition::First))
            && line.contains("!! managed by gripsack — edit the module, not this block !!")
        {
            Ok(scanner::LineKind::LegacyHeader)
        } else {
            Ok(kind)
        }
    })
    .map_err(MergeParseError::from)
}

impl From<scanner::ScanFailure<MergeParseError>> for MergeParseError {
    fn from(failure: scanner::ScanFailure<MergeParseError>) -> Self {
        match failure {
            scanner::ScanFailure::Metadata(error) => error,
            scanner::ScanFailure::NestedOpening { line } => Self {
                line,
                reason: "nested or interleaved managed opener",
            },
            scanner::ScanFailure::MissingOpening { line } => Self {
                line,
                reason: "closing marker has no opener",
            },
            scanner::ScanFailure::DifferentModule { line } => Self {
                line,
                reason: "closing marker names a different module",
            },
            scanner::ScanFailure::MissingClosing { line } => Self {
                line,
                reason: "opening marker has no closing marker; following text is unowned",
            },
        }
    }
}
