//! Interactive consumer execution through the same admitted image, exec
//! payload, descriptor policy and working-directory admission as the piped
//! path — never an ad hoc unbounded `std::process` runner.
//!
//! Standard descriptors are inherited rather than piped, so byte/deadline
//! budgets do not apply to the session: the receipt records zero captured
//! stream limits and a zero session budget, while spawn and cleanup stay
//! bounded. The child still launches in a new process group. When stdin is
//! a terminal the supervisor hands that terminal's foreground to the child
//! group and relays stop/continue so shell job control keeps working;
//! process-control signals delivered to the supervisor are forwarded to the
//! child group. Supervision failures kill the group and reap within a
//! bounded cleanup budget. Descendants that deliberately leave the group
//! remain outside cleanup, exactly as documented for the piped path.
use super::{
    Enforcement, NativeOutcome, ProcessDisposition, ProcessReceipt, ProcessRole, exec_payload,
    invocation::Invocation, sys,
};
use std::{
    ffi::OsStr,
    io,
    os::{
        fd::AsRawFd,
        unix::process::{CommandExt, ExitStatusExt},
    },
    process::{Command, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// Process-control signals the supervisor observes while a consumer runs.
/// Terminal-generated ones reach the foreground child group directly; these
/// handlers exist for programmatic delivery to the supervisor itself.
const OBSERVED_SIGNALS: [libc::c_int; 6] = [
    libc::SIGHUP,
    libc::SIGINT,
    libc::SIGQUIT,
    libc::SIGTERM,
    libc::SIGTSTP,
    libc::SIGCONT,
];
/// Signal bitmask word; every observed signal number is below 64 on the
/// supported Unix targets.
static RECEIVED: AtomicU64 = AtomicU64::new(0);
/// Supervisor wait tick: bounds signal-forwarding latency without spinning.
const WAIT_TICK: Duration = Duration::from_millis(25);
/// Cleanup after a supervision failure gets its own bounded kill/reap budget;
/// an interactive session itself has no deadline to inherit one from.
const CLEANUP_BUDGET: Duration = Duration::from_secs(2);

extern "C" fn record(signal: libc::c_int) {
    RECEIVED.fetch_or(1u64 << signal, Ordering::Relaxed);
}

/// Installed handlers plus the dispositions they replace. Restoration happens
/// exactly once, on drop, in reverse signal order.
struct SignalWatch {
    saved: Vec<(libc::c_int, libc::sigaction)>,
}
impl SignalWatch {
    fn install() -> io::Result<Self> {
        RECEIVED.store(0, Ordering::Relaxed);
        let mut saved = Vec::with_capacity(OBSERVED_SIGNALS.len());
        for &signal in &OBSERVED_SIGNALS {
            // SAFETY: zeroed sigaction is fully initialized below before use.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = record as extern "C" fn(libc::c_int) as usize;
            // SAFETY: valid sigset_t object owned by this call.
            unsafe { libc::sigemptyset(&mut action.sa_mask) };
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            // SAFETY: both pointers name live sigaction objects; `previous`
            // captures the disposition being replaced for later restoration.
            if unsafe { libc::sigaction(signal, &action, &mut previous) } < 0 {
                let error = io::Error::last_os_error();
                let watch = Self { saved };
                drop(watch);
                return Err(error);
            }
            saved.push((signal, previous));
        }
        Ok(Self { saved })
    }
    fn take() -> u64 {
        RECEIVED.swap(0, Ordering::Relaxed)
    }
}
impl Drop for SignalWatch {
    fn drop(&mut self) {
        for (signal, previous) in self.saved.drain(..).rev() {
            // SAFETY: `previous` was captured from this same signal's live
            // disposition at install time and is restored unchanged.
            unsafe { libc::sigaction(signal, &previous, std::ptr::null_mut()) };
        }
    }
}

/// Terminal foreground handoff state. Present only when stdin is a terminal.
struct Terminal {
    own_group: libc::pid_t,
    saved: libc::termios,
}
impl Terminal {
    fn claim() -> io::Result<Option<Self>> {
        // SAFETY: isatty has no memory preconditions.
        if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
            return Ok(None);
        }
        // SAFETY: `saved` is a valid termios output object for stdin's tty.
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut saved) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: getpgrp cannot fail for the calling process.
        let own_group = unsafe { libc::getpgrp() };
        Ok(Some(Self { own_group, saved }))
    }
    fn foreground(&self, group: libc::pid_t) -> io::Result<()> {
        loop {
            // SAFETY: stdin is a live terminal (checked at claim); the group
            // is either this supervisor's own or the owned child's.
            if unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, group) } == 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    fn restore(&self) {
        let _ = self.foreground(self.own_group);
        // SAFETY: `saved` was captured from this terminal at claim time.
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSADRAIN, &self.saved) };
    }
}

impl<'a> Invocation<'a> {
    /// Run the admitted program with inherited stdio. Interactive admission
    /// is exclusive to the task (project consumer) role: every other role
    /// keeps the bounded piped contract. The outcome's receipt carries the
    /// exact exit code or terminating signal; nothing is captured.
    pub fn run_interactive(&self, arguments: &[&OsStr]) -> io::Result<NativeOutcome> {
        if self.role != ProcessRole::Task {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "interactive execution is restricted to the task process role",
            ));
        }
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
        let (payload, environment_keys) = exec_payload::ExecPayload::new(
            self.program,
            arguments,
            self.environment,
            self.role,
            None,
            self.overlay.as_ref(),
            confinement.as_ref(),
        )?;
        let mut receipt = ProcessReceipt {
            executable_sha256: self.program.executable.digest,
            script_sha256: self.program.script.as_ref().map(|script| script.digest),
            loader_sha256: self.program.loader_sha256(),
            byte_binding: self.program.executable.binding.clone(),
            enforcement: Enforcement::ProcessGroup,
            environment_keys,
            deadline_millis: 0,
            stdout_limit: 0,
            stderr_limit: 0,
            exit_code: None,
            signal: None,
            disposition: ProcessDisposition::SpawnFailure,
            error: None,
            cleanup_cause: None,
        };
        let mut command = Command::new(&self.program.executable.path);
        command
            .env_clear()
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .process_group(0);
        let descriptors = super::descriptors::DescriptorPolicy::admit()?;
        let directory = self.directory.as_raw_fd();
        let lease_descriptors = self.leases.descriptors();
        let images = self.program.inherited_images();
        // SAFETY: identical to the piped path — retained image/directory
        // owners outlive spawn and supervision; the closure performs only
        // fchdir/Linux landlock/fcntl/close_range/execve and errno reads.
        // macOS Seatbelt is installed by the platform launcher after exec.
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
        let terminal = Terminal::claim()?;
        let watch = SignalWatch::install()?;
        let child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                receipt.error = Some(super::NativeIoError::capture(&error));
                return Ok(NativeOutcome {
                    success: false,
                    stderr: Vec::new(),
                    stderr_truncated: false,
                    receipt,
                });
            }
        };
        let pid = child.id() as libc::pid_t;
        // std::process::Child::drop neither waits nor kills; raw waitpid below
        // owns observation/reaping, mirroring the piped supervisor's contract.
        drop(child);
        let outcome = supervise(pid, terminal.as_ref(), &mut receipt);
        if let Some(terminal) = &terminal {
            terminal.restore();
        }
        drop(watch);
        let success = outcome.is_ok() && receipt.exit_code == Some(0);
        Ok(NativeOutcome {
            success,
            stderr: Vec::new(),
            stderr_truncated: false,
            receipt,
        })
    }
}

/// Wait, forward process-control signals and relay job control. Returns the
/// raw exit status; on supervision failure the group is killed and reaped
/// within CLEANUP_BUDGET and the receipt records the cleanup disposition.
fn supervise(
    pid: libc::pid_t,
    terminal: Option<&Terminal>,
    receipt: &mut ProcessReceipt,
) -> io::Result<i32> {
    if let Some(terminal) = terminal
        && let Err(error) = terminal.foreground(pid)
    {
        cleanup(pid);
        receipt.disposition = ProcessDisposition::Io;
        receipt.error = Some(super::NativeIoError::capture(&error));
        return Err(error);
    }
    let result = wait_loop(pid, terminal, receipt);
    if result.is_err() {
        cleanup(pid);
    }
    result
}

fn wait_loop(
    pid: libc::pid_t,
    terminal: Option<&Terminal>,
    receipt: &mut ProcessReceipt,
) -> io::Result<i32> {
    loop {
        let mut raw: libc::c_int = 0;
        // SAFETY: valid status pointer; the child PID is exclusively owned
        // here (no external reaper, per the crate contract).
        let rc = unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG | libc::WUNTRACED) };
        if rc < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                receipt.disposition = ProcessDisposition::Io;
                receipt.error = Some(super::NativeIoError::capture(&error));
                return Err(error);
            }
        } else if rc == pid {
            if libc::WIFEXITED(raw) || libc::WIFSIGNALED(raw) {
                let status = ExitStatus::from_raw(raw);
                receipt.disposition = ProcessDisposition::Exited;
                receipt.exit_code = status.code();
                receipt.signal = status.signal();
                return Ok(raw);
            }
            if libc::WIFSTOPPED(raw) {
                relay_stop(pid, terminal, receipt)?;
            }
        }
        for signal in observed(SignalWatch::take()) {
            match signal {
                libc::SIGHUP | libc::SIGINT | libc::SIGQUIT | libc::SIGTERM => {
                    // SAFETY: the owned group may already be gone; ESRCH is
                    // the observed-exit race, not an error worth keeping.
                    unsafe { libc::killpg(pid, signal) };
                }
                libc::SIGTSTP => relay_stop(pid, terminal, receipt)?,
                libc::SIGCONT => relay_continue(pid, terminal),
                _ => {}
            }
        }
        sys::poll(&mut [], WAIT_TICK)?;
    }
}

/// The child (or a programmatic SIGTSTP) stopped: hand the terminal back,
/// stop the supervisor, and let the continued handler resume the child.
fn relay_stop(
    pid: libc::pid_t,
    terminal: Option<&Terminal>,
    receipt: &mut ProcessReceipt,
) -> io::Result<()> {
    if let Some(terminal) = terminal
        && let Err(error) = terminal.foreground(terminal.own_group)
    {
        receipt.disposition = ProcessDisposition::Io;
        receipt.error = Some(super::NativeIoError::capture(&error));
        return Err(error);
    }
    // SAFETY: SIGSTOP cannot be blocked or caught; the supervisor stops until
    // its own supervisor continues it, which arrives as SIGCONT below.
    unsafe { libc::kill(libc::getpid(), libc::SIGSTOP) };
    // A SIGCONT may have arrived between the waitpid observation and the
    // stop; drain it so the resume is not lost.
    relay_continue(pid, terminal);
    Ok(())
}

fn relay_continue(pid: libc::pid_t, terminal: Option<&Terminal>) {
    for signal in observed(SignalWatch::take()) {
        if signal != libc::SIGCONT {
            RECEIVED.fetch_or(1u64 << signal, Ordering::Relaxed);
            continue;
        }
        if let Some(terminal) = terminal {
            let _ = terminal.foreground(pid);
        }
        // SAFETY: resuming the owned group; ESRCH is the observed-exit race.
        unsafe { libc::killpg(pid, libc::SIGCONT) };
    }
}

fn observed(mask: u64) -> impl Iterator<Item = libc::c_int> {
    OBSERVED_SIGNALS
        .into_iter()
        .filter(move |signal| mask & (1u64 << signal) != 0)
}

/// Bounded group kill + reap after a supervision failure. Errors cannot be
/// returned from this compensation path; the caller keeps the original one.
fn cleanup(pid: libc::pid_t) {
    let deadline = Instant::now() + CLEANUP_BUDGET;
    let _ = sys::kill_group(pid, deadline);
    loop {
        let mut raw: libc::c_int = 0;
        // SAFETY: same exclusive-ownership argument as the wait loop.
        let rc = unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG) };
        if rc == pid || (rc < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
        {
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        let _ = sys::poll(&mut [], remaining.min(WAIT_TICK));
    }
}
