//! One admission/execution path for supervised native effects. Executable
//! selection and environment capture never consult a repository overlay.
use super::Ruleset;
use super::{
    Control, Enforcement, Limits, OperatorEnvironment, Outcome, ProcessDisposition, ProcessReceipt,
    ProcessRole, Sha256Digest, StopReason, descriptors::DescriptorPolicy, image::SelectedProgram,
};
use std::{
    ffi::OsStr,
    fs::File,
    io,
    num::NonZeroU64,
    os::{
        fd::AsRawFd,
        unix::{
            fs::OpenOptionsExt,
            process::{CommandExt, ExitStatusExt},
        },
    },
    path::Path,
    process::Command,
    time::Instant,
};

pub struct ActivationEnvironment {
    pub intent: Sha256Digest,
    pub attempt: NonZeroU64,
}

pub enum NativeInput<'a> {
    Bytes(&'a [u8]),
    /// The body is supplied to an already selected interpreter on stdin.
    Script(&'a str),
}

pub struct NativeOutcome {
    pub receipt: ProcessReceipt,
    pub success: bool,
    pub stderr: Vec<u8>,
    pub stderr_truncated: bool,
}

pub struct Invocation<'a> {
    program: &'a SelectedProgram,
    environment: &'a OperatorEnvironment,
    role: ProcessRole,
    directory: File,
    limits: Limits,
    deadline: Instant,
    deadline_millis: u64,
    confinement: Option<Ruleset>,
}

impl<'a> Invocation<'a> {
    pub fn admit(
        environment: &'a OperatorEnvironment,
        role: ProcessRole,
        program: &'a SelectedProgram,
        directory: &Path,
        limits: Limits,
    ) -> io::Result<Self> {
        let now = Instant::now();
        let deadline = now.checked_add(limits.timeout).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "native deadline exceeds the clock range",
            )
        })?;
        let deadline = limits
            .operation_deadline
            .map_or(deadline, |outer| outer.min(deadline));
        if deadline <= now {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native admission deadline expired",
            ));
        }
        if !directory.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native working directory must be absolute",
            ));
        }
        let directory = std::fs::canonicalize(directory)?;
        let directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(directory)?;
        let directory = super::descriptors::retain_above_stdio(directory)?;
        let deadline_millis =
            u64::try_from(deadline.duration_since(now).as_millis()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "native deadline receipt overflow",
                )
            })?;
        Ok(Self {
            program,
            environment,
            role,
            directory,
            limits,
            deadline,
            deadline_millis,
            confinement: None,
        })
    }

    /// Confine the launched process (and its descendants) to an assembled
    /// filesystem boundary. Applied in the forked child before descriptors and
    /// exec; assembly errors surface before spawn.
    pub fn confine(mut self, ruleset: Ruleset) -> Self {
        self.confinement = Some(ruleset);
        self
    }

    /// Preserve the shell action's `-c` semantics, with closed bounded stdin.
    /// Script identity is computed before launch; no script/argv bytes enter
    /// the receipt or diagnostic output.
    pub fn run_shell_body(
        &self,
        script: &str,
        activation: &ActivationEnvironment,
        on_bytes: impl FnMut(&[u8]) -> Control,
    ) -> io::Result<NativeOutcome> {
        if script.len() > self.limits.input_bytes.bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "hook script exceeds the input budget",
            ));
        }
        let digest = Sha256Digest::of(script.as_bytes());
        let mut outcome = self.run(
            &[OsStr::new("-c"), OsStr::new(script)],
            NativeInput::Bytes(b""),
            Some(activation),
            on_bytes,
        )?;
        outcome.receipt.script_sha256 = Some(digest);
        Ok(outcome)
    }

    pub fn run(
        &self,
        arguments: &[&OsStr],
        input: NativeInput<'_>,
        activation: Option<&ActivationEnvironment>,
        on_bytes: impl FnMut(&[u8]) -> Control,
    ) -> io::Result<NativeOutcome> {
        if activation.is_some() && self.role != ProcessRole::Hook {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "activation identity is reserved to the hook process role",
            ));
        }
        let (input, input_script) = match input {
            NativeInput::Bytes(bytes) => (bytes, None),
            NativeInput::Script(script) => {
                (script.as_bytes(), Some(Sha256Digest::of(script.as_bytes())))
            }
        };
        let (payload, environment_keys) = super::exec_payload::ExecPayload::new(
            self.program,
            arguments,
            self.environment,
            self.role,
            activation,
        )?;
        let mut command = Command::new(&self.program.executable.path);
        command.env_clear();
        let descriptors = DescriptorPolicy::admit()?;
        let confinement = self
            .confinement
            .as_ref()
            .map(super::Ruleset::try_clone)
            .transpose()?;
        let directory = self.directory.as_raw_fd();
        #[cfg(target_os = "linux")]
        let script = self
            .program
            .script
            .as_ref()
            .map(|image| image.file.as_raw_fd());
        #[cfg(target_os = "macos")]
        let script = None;
        // SAFETY: retained image/directory owners outlive spawn and supervision.
        // The closure performs only fchdir/landlock syscalls/fcntl/close_range/
        // execve and errno reads.
        unsafe {
            command.pre_exec(move || {
                if let Some(ruleset) = &confinement
                    && let Err(error) = ruleset.restrict()
                {
                    return Err(error);
                }
                if libc::fchdir(directory) < 0 {
                    return Err(io::Error::last_os_error());
                }
                descriptors.apply(script)?;
                Err(payload.execute())
            });
        }
        let limits = Limits {
            operation_deadline: Some(self.deadline),
            ..self.limits
        };
        let mut receipt = ProcessReceipt {
            executable_sha256: self.program.executable.digest,
            script_sha256: input_script
                .or_else(|| self.program.script.as_ref().map(|script| script.digest)),
            byte_binding: self.program.executable.binding.clone(),
            enforcement: Enforcement::ProcessGroup,
            environment_keys,
            deadline_millis: self.deadline_millis,
            stdout_limit: limits.stdout_bytes.bytes(),
            stderr_limit: limits.stderr_bytes.bytes(),
            exit_code: None,
            signal: None,
            disposition: ProcessDisposition::SpawnFailure,
            error: None,
            cleanup_cause: None,
        };
        let (success, stderr, stderr_truncated) =
            match super::run_raw(&mut command, input, limits, on_bytes) {
                Ok(Outcome {
                    status,
                    reason,
                    stderr,
                    stderr_truncated,
                }) => {
                    let success = matches!(reason, StopReason::Exited)
                        && status.is_some_and(|status| status.success());
                    receipt.disposition = disposition(&reason);
                    receipt.exit_code = status.and_then(|status| status.code());
                    receipt.signal = status.and_then(|status| status.signal());
                    receipt.error = match &reason {
                        StopReason::Io(error) | StopReason::Cleanup { error, .. } => {
                            Some(super::NativeIoError::capture(error))
                        }
                        _ => None,
                    };
                    if let StopReason::Cleanup { cause, .. } = &reason {
                        receipt.cleanup_cause = Some(disposition(cause));
                    }
                    (success, stderr, stderr_truncated)
                }
                Err(error) => {
                    receipt.error = Some(super::NativeIoError::capture(&error));
                    (false, Vec::new(), false)
                }
            };
        Ok(NativeOutcome {
            success,
            stderr,
            stderr_truncated,
            receipt,
        })
    }
}

fn disposition(reason: &StopReason) -> ProcessDisposition {
    match reason {
        StopReason::Exited => ProcessDisposition::Exited,
        StopReason::Deadline => ProcessDisposition::Deadline,
        StopReason::InputLimit => ProcessDisposition::InputLimit,
        StopReason::LineLimit => ProcessDisposition::LineLimit,
        StopReason::StdoutLimit => ProcessDisposition::StdoutLimit,
        StopReason::StderrLimit => ProcessDisposition::StderrLimit,
        StopReason::Io(_) => ProcessDisposition::Io,
        StopReason::Cleanup { .. } => ProcessDisposition::CleanupFailure,
    }
}
