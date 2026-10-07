//! Runtime discovery uses the same bounded, retained-image supervisor as other
//! native calls, with only a fixed system-tool environment and no credentials.
use crate::{Control, Invocation, Limits, NativeInput, NativeOutcome, OperatorEnvironment,
    ProcessDisposition, ProcessRole, SelectedProgram, Sha256Digest};
use std::{ffi::OsStr, io, path::Path, time::Instant};

/// FHS system helper search only; never inherit the operator/repository PATH.
const SYSTEM_HELPER_PATH: &str = "/usr/bin:/bin";

pub(super) fn run(
    program: &Path,
    expected: Option<Sha256Digest>,
    cwd: &Path,
    arguments: &[&OsStr],
) -> io::Result<(NativeOutcome, Vec<u8>)> {
    let environment = OperatorEnvironment::admit([
        ("PATH".into(), SYSTEM_HELPER_PATH.into()),
        ("LC_ALL".into(), "C".into()),
    ])?;
    let limits = Limits::default();
    let deadline = Instant::now().checked_add(limits.timeout)
        .ok_or_else(|| io::Error::other("runtime discovery deadline overflow"))?;
    let selected = SelectedProgram::select(&environment, program, expected, deadline)?;
    let limits = Limits { operation_deadline: Some(deadline), ..limits };
    let invocation = Invocation::admit(&environment, ProcessRole::Fact, &selected, cwd, limits)?;
    let mut stdout = Vec::new();
    let outcome = invocation.run(arguments, NativeInput::Bytes(b""), None, |bytes| {
        stdout.extend_from_slice(bytes);
        Control::Continue
    })?;
    if outcome.receipt.disposition != ProcessDisposition::Exited {
        return Err(io::Error::other(format!(
            "runtime discovery did not exit normally ({:?})", outcome.receipt.disposition
        )));
    }
    Ok((outcome, stdout))
}
