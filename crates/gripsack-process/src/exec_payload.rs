//! Exec exactly the admitted image. std::Command's Unix execvp fallback may
//! interpret ENOEXEC through an unadmitted shell; this path deliberately uses
//! execve after the shared supervisor has configured stdio/process groups.
use super::{
    ActivationEnvironment, OperatorEnvironment, ProcessRole, image::SelectedProgram,
    overlay::EnvironmentOverlay,
};
use std::{
    ffi::{CString, OsStr, OsString},
    io,
    os::unix::ffi::OsStrExt,
};

const EXEC_VECTOR_BYTES: usize = 4 * 1024 * 1024;

struct VectorBudget {
    bytes: usize,
}
impl VectorBudget {
    fn admit(&mut self, length: usize) -> io::Result<()> {
        self.bytes = self
            .bytes
            .checked_add(length)
            .and_then(|bytes| bytes.checked_add(1 + std::mem::size_of::<*const libc::c_char>()))
            .filter(|bytes| *bytes <= EXEC_VECTOR_BYTES)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "native argument/environment vector exceeds its budget",
                )
            })?;
        Ok(())
    }
}

pub(crate) struct ExecPayload {
    program: CString,
    _arguments: Vec<CString>,
    _environment: Vec<CString>,
    arguments: Vec<*const libc::c_char>,
    environment: Vec<*const libc::c_char>,
}
// SAFETY: all pointers refer to immutable CString allocations owned by this
// same object. Moving the vectors/object does not relocate those allocations.
// No pointer is exposed or mutated; the payload outlives its pre_exec call.
unsafe impl Send for ExecPayload {}
unsafe impl Sync for ExecPayload {}

impl ExecPayload {
    pub(crate) fn new(
        program: &SelectedProgram,
        arguments: &[&OsStr],
        operator: &OperatorEnvironment,
        role: ProcessRole,
        activation: Option<&ActivationEnvironment>,
        overlay: Option<&EnvironmentOverlay>,
        confinement: Option<&super::Ruleset>,
    ) -> io::Result<(Self, Vec<String>)> {
        #[cfg(target_os = "macos")]
        let execution = if confinement.is_some() {
            OsStr::new(super::confinement::seatbelt::LAUNCHER)
        } else {
            program.execution_image().path.as_os_str()
        };
        #[cfg(not(target_os = "macos"))]
        let execution = program.execution_image().path.as_os_str();
        #[cfg(target_os = "macos")]
        let initial_arguments = confinement
            .map(|boundary| {
                boundary.macos_launch_arguments(
                    program.execution_argument_zero(),
                    program.execution_image().path.as_os_str(),
                )
            })
            .into_iter()
            .flatten()
            .chain(
                confinement
                    .is_none()
                    .then_some(program.execution_argument_zero()),
            );
        #[cfg(not(target_os = "macos"))]
        let initial_arguments = {
            let _ = confinement;
            std::iter::once(program.execution_argument_zero())
        };
        let arguments = initial_arguments
            .chain(program.loader_arguments())
            .chain(program.interpreter_argument.as_deref())
            .chain(
                program
                    .script
                    .as_ref()
                    .map(|script| script.path.as_os_str()),
            )
            .chain(arguments.iter().copied());
        let intent = activation.map(|identity| identity.intent.to_string());
        let attempt = activation.map(|identity| identity.attempt.to_string());
        let reserved = intent
            .as_deref()
            .map(|value| {
                (
                    OsStr::new("GRIPSACK_ACTIVATION_INTENT_ID"),
                    OsStr::new(value),
                )
            })
            .into_iter()
            .chain(
                attempt
                    .as_deref()
                    .map(|value| (OsStr::new("GRIPSACK_ACTIVATION_ATTEMPT"), OsStr::new(value))),
            );
        let mut operator_entries: Vec<(OsString, OsString)> = match overlay {
            Some(overlay) => overlay.merge(operator.entries(role)).into_iter().collect(),
            None => operator
                .entries(role)
                .map(|(key, value)| (key.to_os_string(), value.to_os_string()))
                .collect(),
        };
        program.admit_gnu_environment(&operator_entries)?;
        if let Some(libraries) = &program.macho_libraries {
            if operator_entries
                .iter()
                .any(|(key, _)| key.as_bytes().starts_with(b"DYLD_"))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "caller DYLD environment conflicts with the admitted Mach-O lookup plan",
                ));
            }
            operator_entries.push(("DYLD_LIBRARY_PATH".into(), libraries.clone()));
            for key in [
                "DYLD_FALLBACK_LIBRARY_PATH",
                "DYLD_FRAMEWORK_PATH",
                "DYLD_FALLBACK_FRAMEWORK_PATH",
            ] {
                operator_entries.push((key.into(), "/dev/null".into()));
            }
        }
        let environment = operator_entries
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_os_str()))
            .chain(reserved);
        let mut budget = VectorBudget {
            bytes: 2 * std::mem::size_of::<*const libc::c_char>(),
        };
        budget.admit(execution.as_bytes().len())?;
        let mut argument_count = 0;
        for argument in arguments.clone() {
            budget.admit(argument.as_bytes().len())?;
            argument_count += 1;
        }
        let mut environment_count = 0;
        for (key, value) in environment.clone() {
            let length = key
                .as_bytes()
                .len()
                .checked_add(value.as_bytes().len())
                .and_then(|length| length.checked_add(1))
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "native environment size overflow",
                    )
                })?;
            budget.admit(length)?;
            environment_count += 1;
        }
        let mut owned_arguments = Vec::with_capacity(argument_count);
        for argument in arguments {
            owned_arguments.push(c_string(argument.as_bytes())?);
        }
        let mut owned_environment = Vec::with_capacity(environment_count);
        let mut keys = Vec::with_capacity(environment_count);
        for (key, value) in environment {
            let mut bytes = Vec::with_capacity(key.as_bytes().len() + value.as_bytes().len() + 2);
            bytes.extend_from_slice(key.as_bytes());
            bytes.push(b'=');
            bytes.extend_from_slice(value.as_bytes());
            owned_environment.push(CString::new(bytes).map_err(|_| invalid_nul())?);
            keys.push(key.to_string_lossy().into_owned());
        }
        keys.sort_unstable();
        let mut arguments = Vec::with_capacity(owned_arguments.len() + 1);
        arguments.extend(owned_arguments.iter().map(|argument| argument.as_ptr()));
        arguments.push(std::ptr::null());
        let mut environment = Vec::with_capacity(owned_environment.len() + 1);
        environment.extend(owned_environment.iter().map(|value| value.as_ptr()));
        environment.push(std::ptr::null());
        Ok((
            Self {
                program: c_string(execution.as_bytes())?,
                _arguments: owned_arguments,
                _environment: owned_environment,
                arguments,
                environment,
            },
            keys,
        ))
    }

    /// Called only in the forked child after descriptor/working-directory
    /// admission. Success replaces that child and never returns.
    pub(crate) fn execute(&self) -> io::Error {
        // SAFETY: all three pointers are terminated arrays/strings backed by
        // this live immutable owner; execve consumes them synchronously.
        unsafe {
            libc::execve(
                self.program.as_ptr(),
                self.arguments.as_ptr(),
                self.environment.as_ptr(),
            );
        }
        io::Error::last_os_error()
    }
}

fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| invalid_nul())
}
fn invalid_nul() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "native argument or environment contains NUL",
    )
}
