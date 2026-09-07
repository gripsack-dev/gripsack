//! Bounded, single-threaded Unix process supervision (Linux and macOS).
//!
//! `timeout` includes cleanup: the final min(timeout / 4, 2 seconds) is
//! reserved for termination, draining and reaping. Thus Deadline can be
//! selected before the total budget expires. The clock starts before spawn.
//! Spawn/exec, synchronous callbacks, allocation and kernel scheduling cannot
//! be preempted here. Callbacks must return promptly. Timely SIGKILL delivery
//! and scheduler progress are required to reap within the budget; otherwise
//! Cleanup is returned with status None. There is no background reaper.
//!
//! The caller must not reap this child (including via a SIGCHLD handler), set
//! SIGCHLD to SIG_IGN/SA_NOCLDWAIT, or change its group in a pre_exec hook.
//! waitid(WNOWAIT) preserves the leader's PID until all group signals are done.
//! Concurrent external reaping would invalidate that guarantee. Descendants
//! that deliberately leave the group cannot be killed by this mechanism;
//! their inherited pipes are closed at the deadline instead.

mod exchange;
mod input;
pub use input::InputBuffer;
mod lifecycle;
mod sys;
#[cfg(test)]
mod tests;

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Continue,
    /// Suppress subsequent callbacks, not framing, limits, input or exit checks.
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub timeout: Duration,
    pub input_bytes: usize,
    /// Content bytes, excluding LF. CR is ordinary content.
    pub line_bytes: usize,
    /// Includes delimiters and bytes received after Response.
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
    pub retained_stderr_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            input_bytes: 4 * 1024 * 1024,
            line_bytes: 1024 * 1024,
            stdout_bytes: 16 * 1024 * 1024,
            stderr_bytes: 16 * 1024 * 1024,
            retained_stderr_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug)]
pub enum StopReason {
    /// Leader exited and the pipes drained; not a protocol-success verdict.
    Exited,
    Deadline,
    InputLimit,
    LineLimit,
    StdoutLimit,
    StderrLimit,
    Io(io::Error),
    Cleanup {
        cause: Box<StopReason>,
        error: io::Error,
    },
}

impl StopReason {
    pub fn cause(&self) -> Option<&StopReason> {
        match self {
            Self::Cleanup { cause, .. } => Some(cause),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct Outcome {
    /// Present exactly when this supervisor successfully reaped the leader.
    pub status: Option<ExitStatus>,
    pub reason: StopReason,
    /// Last retained_stderr_bytes bytes actually read, including a cap-crossing read.
    pub stderr: Vec<u8>,
}

/// Force piped stdio and a new child process group, incrementally send input,
/// and deliver LF-stripped stdout lines (including a final nonempty fragment).
/// Empty LF-terminated lines are delivered; EOF alone is not an empty line.
/// Response only suppresses callbacks; input is still sent and closed normally.
/// EPIPE means the peer declined further input, not a supervisor I/O failure.
///
/// Oversized input and zero timeout do not spawn. Err is reserved for spawn
/// failure or a timeout unrepresentable by Instant. Post-spawn failures are
/// Outcomes. The command is mutated; callers supply any arguments/environment.
/// On leader exit, kill the remaining owned group immediately, even if its
/// descendants have closed their pipes. Reap only after the last group signal.
/// A panic in the callback unwinds through a bounded kill/reap guard.
pub fn run(
    command: &mut Command,
    input: &[u8],
    limits: Limits,
    mut on_line: impl FnMut(&[u8]) -> Control,
) -> io::Result<Outcome> {
    let empty = |reason| Outcome {
        status: None,
        reason,
        stderr: Vec::new(),
    };
    if input.len() > limits.input_bytes {
        return Ok(empty(StopReason::InputLimit));
    }
    let start = Instant::now();
    let end = start.checked_add(limits.timeout).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "timeout exceeds Instant range")
    })?;
    if limits.timeout.is_zero() {
        return Ok(empty(StopReason::Deadline));
    }
    let reserve = (limits.timeout / 4).min(Duration::from_secs(2));
    let active_end = end - reserve;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn()?;
    // Install before any post-spawn fallible operation. Child::drop does not wait.
    let mut guard = lifecycle::Guard::new(child.id(), end);
    let mut exchange = exchange::Exchange::new(&mut child, input, limits);
    let reason = match exchange.configure() {
        Ok(()) => exchange.drive(&mut guard, active_end, &mut on_line),
        Err(error) => StopReason::Io(error),
    };
    exchange.close_input();
    let (status, cleanup_error) = guard.finish(|remaining| exchange.drain(remaining));
    let reason = match cleanup_error {
        Some(error) => StopReason::Cleanup {
            cause: Box::new(reason),
            error,
        },
        None => reason,
    };
    Ok(Outcome {
        status,
        reason,
        stderr: exchange.into_tail(),
    })
}
