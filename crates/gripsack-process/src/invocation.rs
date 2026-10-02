//! One admission/execution path for supervised native effects. Executable
//! selection and environment capture never consult a repository overlay.
use super::Ruleset;
use super::{
    Control, Enforcement, Limits, OperatorEnvironment, Outcome, ProcessDisposition, ProcessReceipt,
    ProcessRole, Sha256Digest, StopReason, descriptors::DescriptorPolicy, image::SelectedProgram,
    overlay::EnvironmentOverlay,
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
    pub(crate) program: &'a SelectedProgram,
    pub(crate) environment: &'a OperatorEnvironment,
    pub(crate) role: ProcessRole,
    pub(crate) directory: File,
    pub(crate) limits: Limits,
    pub(crate) deadline: Instant,
    pub(crate) deadline_millis: u64,
    pub(crate) confinement: Option<Ruleset>,
    pub(crate) leases: super::ProcessLeases,
    pub(crate) overlay: Option<EnvironmentOverlay>,
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
            leases: super::ProcessLeases::default(),
            overlay: None,
        })
    }

    /// Layer an explicitly admitted repository-owned overlay over the operator
    /// snapshot. The merge is deterministic (overlay wins declared keys, its
    /// search prefix precedes the operator PATH) and happens at payload
    /// construction, never by mutating the operator boundary.
    pub fn with_overlay(mut self, overlay: EnvironmentOverlay) -> Self {
        self.overlay = Some(overlay);
        self
    }

    /// Confine the launched process (and its descendants) to an assembled
    /// filesystem boundary. Applied in the forked child before descriptors and
    /// exec; assembly errors surface before spawn.
    pub fn confine(mut self, ruleset: Ruleset) -> Self {
        self.confinement = Some(ruleset);
        self
    }

    /// Grant only the two explicit coordination handles of a build/verification
    /// attempt or a consumer process root. All unrelated descriptors remain
    /// close-on-exec. A retained worker/root lock survives parent death while
    /// the bridge or the launched consumer still holds its duplicated handle.
    pub fn retain_leases(mut self, leases: super::ProcessLeases) -> io::Result<Self> {
        if !matches!(
            self.role,
            ProcessRole::Build | ProcessRole::Verify | ProcessRole::Task
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "coordination leases are restricted to build, verification and task roles",
            ));
        }
        self.leases = leases.admit()?;
        Ok(self)
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
        let confinement = self
            .confinement
            .as_ref()
            .map(super::Ruleset::try_clone)
            .transpose()?;
        #[cfg(target_os = "macos")]
        let confinement = confinement
            .map(|ruleset| {
                let ruleset = ruleset.granting_image_read(&self.program.executable.path)?;
                match self.program.script.as_ref() {
                    Some(script) => ruleset.granting_image_read(&script.path),
                    None => Ok(ruleset),
                }
            })
            .transpose()?;
        let (payload, environment_keys) = super::exec_payload::ExecPayload::new(
            self.program,
            arguments,
            self.environment,
            self.role,
            activation,
            self.overlay.as_ref(),
            confinement.as_ref(),
        )?;
        let mut command = Command::new(&self.program.executable.path);
        command.env_clear();
        let descriptors = DescriptorPolicy::admit()?;
        let directory = self.directory.as_raw_fd();
        let lease_descriptors = self.leases.descriptors();
        let images = self.program.inherited_images();
        // SAFETY: retained image/directory owners outlive spawn and supervision.
        // The closure performs only fchdir, Linux Landlock syscalls,
        // fcntl/close_range, execve and errno reads. macOS Seatbelt is installed
        // by the trusted platform launcher AFTER this exec.
        unsafe {
            command.pre_exec(move || {
                #[cfg(target_os = "linux")]
                if let Some(ruleset) = &confinement
                    && let Err(error) = ruleset.restrict()
                {
                    return Err(error);
                }
                if libc::fchdir(directory) < 0 {
                    return Err(io::Error::last_os_error());
                }
                descriptors.apply(images, lease_descriptors)?;
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
            loader_sha256: self.program.loader_sha256(),
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
