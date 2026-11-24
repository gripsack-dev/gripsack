//! Read-only runtime probe. Do not run workspace code, dependency-discovery
//! tools, a shell command, or an implicit PATH fallback to obtain a version.
use gripsack_host::process::{
    Boundary, Control, Invocation, Limits, NativeInput, OperatorEnvironment, ProcessDisposition,
    ProcessRole, RetainedStderrLimit, Ruleset, SelectedProgram, StderrByteLimit, StdoutByteLimit,
};
use std::ffi::OsStr;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) fn version(path: &Path) -> io::Result<String> {
    if !path.is_absolute() {
        return Err(io::Error::other(
            "diagnosis requires an absolute evaluator path",
        ));
    }
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::other("not an executable regular file"));
    }
    let environment = OperatorEnvironment::admit([])?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let program = SelectedProgram::select(&environment, path, None, deadline)?;
    // Only system loader/library trees, no home, repository, scratch or
    // writable root. An override's output is data, not a diagnostic to echo.
    let mut boundary = Boundary::new();
    for dir in ["/lib", "/lib64", "/usr/lib", "/usr/lib64"] {
        boundary = boundary.read_beneath_optional(Path::new(dir))?;
    }
    let cache = Path::new("/etc/ld.so.cache");
    if cache.try_exists()? {
        boundary = boundary.read_file(cache)?;
    }
    let ruleset = Ruleset::assemble(&boundary, program.is_script())?;
    let limits = Limits {
        timeout: Duration::from_secs(3),
        operation_deadline: Some(deadline),
        stdout_bytes: StdoutByteLimit::new(4096),
        stderr_bytes: StderrByteLimit::new(4096),
        retained_stderr_bytes: RetainedStderrLimit::new(4096),
        ..Limits::default()
    };
    let mut output = Vec::new();
    let outcome = Invocation::admit(
        &environment,
        ProcessRole::Probe,
        &program,
        Path::new("/"),
        limits,
    )?
    .confine(ruleset)
    .run(
        &[OsStr::new("--version")],
        NativeInput::Bytes(b""),
        None,
        |bytes| {
            output.extend_from_slice(bytes);
            Control::Continue
        },
    )?;
    if outcome.receipt.disposition != ProcessDisposition::Exited {
        return Err(io::Error::other(format!(
            "--version stopped: {:?}",
            outcome.receipt.disposition
        )));
    }
    if !outcome.success {
        return Err(io::Error::other(match outcome.receipt.exit_code {
            Some(code) => format!("--version exited with status {code}"),
            None => "--version terminated without a successful exit".into(),
        }));
    }
    let version = std::str::from_utf8(&output)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.strip_prefix("deno "))
        .and_then(|text| text.split_ascii_whitespace().next())
        .filter(|version| {
            let release = version.split(['-', '+']).next().unwrap_or_default();
            let mut numbers = release.split('.').map(str::parse::<u64>);
            matches!(
                (
                    numbers.next(),
                    numbers.next(),
                    numbers.next(),
                    numbers.next()
                ),
                (Some(Ok(2..)), Some(Ok(_)), Some(Ok(_)), None)
            ) && version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
        })
        .ok_or_else(|| io::Error::other("--version did not report a supported deno 2+ version"))?;
    Ok(version.to_owned())
}
