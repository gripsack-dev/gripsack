//! `#!` interpreter-line metadata: program plus its optional single
//! argument. Classification reports the line; admission layers decide
//! whether the program is an acceptable interpreter.
use super::{ExecutableFormat, ExecutableMetadata, Interpreter, LayoutError, MAX_STRING_BYTES};
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

pub(super) fn shebang(header: &[u8]) -> Result<ExecutableMetadata, LayoutError> {
    let line = match header.iter().position(|byte| matches!(*byte, b'\n' | b'\0')) {
        Some(end) => &header[..end],
        None if (header.len() as u64) < MAX_STRING_BYTES => header,
        None => return Err(LayoutError::Malformed("unterminated interpreter line")),
    };
    // binfmt_script trims only spaces and tabs. In particular CR is part of
    // a name/argument, and the optional argument is one unsplit string.
    let text = &line[2..];
    let start = text.iter().position(|byte| !matches!(*byte, b' ' | b'\t')).unwrap_or(text.len());
    let text = &text[start..];
    let end = text.iter().rposition(|byte| !matches!(*byte, b' ' | b'\t')).map_or(0, |index| index + 1);
    let text = &text[..end];
    let split = text
        .iter()
        .position(|byte| matches!(*byte, b' ' | b'\t'))
        .unwrap_or(text.len());
    let program = OsString::from_vec(text[..split].to_vec());
    let argument = text[split..]
        .iter()
        .position(|byte| !matches!(*byte, b' ' | b'\t'))
        .map(|start| OsString::from_vec(text[split + start..].to_vec()));
    if program.is_empty() {
        return Err(LayoutError::Malformed("empty interpreter"));
    }
    Ok(ExecutableMetadata {
        format: Some(ExecutableFormat::Script),
        interpreter: Some(Interpreter::Shebang { program, argument }),
        ..ExecutableMetadata::default()
    })
}
