//! Loader controls are capabilities of the selected bytes, not a libc version.
//! Enterprise GNU runtimes backport these controls without changing 2.28.
use super::super::{Image, SelectedProgram};
use crate::{
    Control, InputByteLimit, Invocation, Limits, NativeInput, OperatorEnvironment,
    ProcessDisposition, ProcessRole, RetainedStderrLimit, StderrByteLimit, StdoutByteLimit,
    executable::{self, ExecutableFormat},
};
use std::{
    ffi::OsStr,
    io::{self, Seek},
    path::{Path, PathBuf},
    time::Instant,
};

/// A GNU interpreter whose required controls were exercised on these exact
/// sealed bytes. It cannot be constructed from a version string or serialized
/// capability assertion. The original platform path remains the PT_INTERP
/// authority; execution reuses the retained image rather than reopening it.
pub struct SelectedGnuLoader {
    pub(in crate::image) image: Image,
    path: PathBuf,
}

impl SelectedGnuLoader {
    pub fn select(path: &Path, deadline: Instant) -> io::Result<Self> {
        if !cfg!(target_os = "linux") || !path.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "GNU loader selection requires an absolute Linux platform interpreter",
            ));
        }
        super::admit_system_preload(Path::new("/etc/ld.so.preload"))?;
        // No operator/repository loader variables, locale, credentials or PATH
        // participate in this platform capability measurement.
        let environment = OperatorEnvironment::admit(std::iter::empty())?;
        let selected = SelectedProgram::select(&environment, path, None, deadline)?;
        let mut reader = selected.executable.file.try_clone()?;
        reader.rewind()?;
        let metadata = executable::classify(&mut reader)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if selected.is_script()
            || metadata.format != Some(ExecutableFormat::Elf)
            || metadata.interpreter.is_some()
            || !metadata.needed_libraries.is_empty()
            || !metadata.elf_loader_extensions.is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected GNU interpreter is not a standalone ELF loader",
            ));
        }
        probe(&selected, &environment, deadline)?;
        Ok(Self {
            image: selected.executable,
            path: path.to_owned(),
        })
    }

    /// The platform interpreter spelling admitted by PT_INTERP. Persistent
    /// command projections use this path under the existing OS-runtime
    /// integrity assumption; supervised launches use the sealed image.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Explicit workspace values cannot re-enable auditing, preload objects,
    /// hwcap masks or tunables after closure admission. LD_LIBRARY_PATH alone
    /// is composed separately, beneath the explicit loader search argument.
    pub fn check_environment_key(&self, key: &OsStr) -> io::Result<()> {
        use std::os::unix::ffi::OsStrExt;
        if (key.as_bytes().starts_with(b"LD_") && key != OsStr::new("LD_LIBRARY_PATH"))
            || key == OsStr::new("GLIBC_TUNABLES")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "declared environment {key:?} conflicts with the admitted GNU loader policy"
                ),
            ));
        }
        Ok(())
    }
}

fn probe(
    selected: &SelectedProgram,
    environment: &OperatorEnvironment,
    deadline: Instant,
) -> io::Result<()> {
    const HELP_BYTES: u64 = 32 * 1024;
    const CONTROLS: [&str; 5] = [
        "--inhibit-cache",
        "--glibc-hwcaps-mask",
        "--inhibit-rpath",
        "--library-path",
        "--argv0",
    ];
    let invocation = Invocation::admit(
        environment,
        ProcessRole::Fact,
        selected,
        Path::new("/"),
        Limits {
            operation_deadline: Some(deadline),
            input_bytes: InputByteLimit::new(0),
            stdout_bytes: StdoutByteLimit::new(HELP_BYTES),
            stderr_bytes: StderrByteLimit::new(HELP_BYTES),
            retained_stderr_bytes: RetainedStderrLimit::new(HELP_BYTES as usize),
            ..Limits::default()
        },
    )?;
    let mut help = Vec::new();
    // Exercise option parsing, not just strings found in a help message. No
    // application is loaded by --help; no package code runs during admission.
    let arguments = [
        "--inhibit-cache",
        "--glibc-hwcaps-mask",
        "",
        "--inhibit-rpath",
        "",
        "--library-path",
        "/nonexistent",
        "--argv0",
        "gripsack-loader-probe",
        "--help",
    ]
    .map(OsStr::new);
    let outcome = invocation.run(&arguments, NativeInput::Bytes(b""), None, |bytes| {
        help.extend_from_slice(bytes);
        Control::Continue
    })?;
    if !outcome.success {
        let kind = match outcome.receipt.disposition {
            ProcessDisposition::Exited => io::ErrorKind::Unsupported,
            ProcessDisposition::Deadline => io::ErrorKind::TimedOut,
            _ => io::ErrorKind::Other,
        };
        return Err(io::Error::new(
            kind,
            format!(
                "selected GNU loader cannot establish required controls ({}, {}, {}, {}, {}): {:?}: {}",
                CONTROLS[0],
                CONTROLS[1],
                CONTROLS[2],
                CONTROLS[3],
                CONTROLS[4],
                outcome.receipt.disposition,
                String::from_utf8_lossy(&outcome.stderr).trim(),
            ),
        ));
    }
    let help = std::str::from_utf8(&help).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "selected GNU loader capability output is not UTF-8",
        )
    })?;
    for control in CONTROLS {
        if !help
            .lines()
            .any(|line| line.split_ascii_whitespace().next() == Some(control))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("selected GNU loader does not advertise required control {control}"),
            ));
        }
    }
    Ok(())
}
