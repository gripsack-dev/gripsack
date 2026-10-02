//! Bounded informational Git reads belong to CLI preparation, not store IO.
use gripsack_process::{
    Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessRole,
    RetainedStderrLimit, SelectedProgram, StderrByteLimit, StdoutByteLimit,
};
use gripsack_store::trust::GitProvenance;
use std::{
    ffi::OsStr,
    path::Path,
    time::{Duration, Instant},
};

pub(super) fn capture(repo: &Path, environment: &OperatorEnvironment) -> GitProvenance {
    let collect = || -> std::io::Result<GitProvenance> {
        let timeout = Duration::from_secs(10);
        let deadline = Instant::now() + timeout;
        let selected = SelectedProgram::select(environment, Path::new("git"), None, deadline)?;
        let invocation = Invocation::admit(
            environment,
            ProcessRole::Fact,
            &selected,
            repo,
            Limits {
                timeout,
                operation_deadline: Some(deadline),
                stdout_bytes: StdoutByteLimit::new(16 * 1024),
                stderr_bytes: StderrByteLimit::new(16 * 1024),
                retained_stderr_bytes: RetainedStderrLimit::new(0),
                ..Limits::default()
            },
        )?;
        let remote = query(&invocation, ["remote", "get-url", "origin"]);
        let commit = query(&invocation, ["rev-parse", "HEAD"]);
        Ok(GitProvenance::from_git(remote.as_deref(), commit))
    };
    // Missing git or unavailable informational metadata never invents a fact.
    // Approval still depends on captured bytes and the actual grant policy.
    collect().unwrap_or_default()
}

fn query<const N: usize>(invocation: &Invocation<'_>, arguments: [&str; N]) -> Option<String> {
    let arguments = arguments.map(OsStr::new);
    let mut output = Vec::new();
    let outcome = invocation
        .run(&arguments, NativeInput::Bytes(b""), None, |bytes| {
            output.extend_from_slice(bytes);
            Control::Continue
        })
        .ok()?;
    if !outcome.success {
        return None;
    }
    let mut output = String::from_utf8(output).ok()?;
    output.truncate(output.trim_end().len());
    let leading = output.len() - output.trim_start().len();
    drop(output.drain(..leading));
    (!output.is_empty()).then_some(output)
}
