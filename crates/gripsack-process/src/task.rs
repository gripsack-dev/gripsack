//! Finite consumer signal and terminal ownership. Stream budgets and kill/reap
//! stay in Exchange/Guard; this module never reads or waits for inherited stdin.
use super::{
    StopReason,
    interactive::{SignalWatch, observed},
    lifecycle::Guard,
};
use std::io;

pub(super) struct Session {
    // Keep the watcher alive through group cleanup and terminal restoration.
    _watch: SignalWatch,
    terminal: Option<Terminal>,
}

impl Session {
    pub(super) fn new(inherit_stdin: bool) -> io::Result<Self> {
        let terminal = if inherit_stdin {
            Terminal::claim()?
        } else {
            None
        };
        Ok(Self {
            _watch: SignalWatch::install()?,
            terminal,
        })
    }

    pub(super) fn enter(&mut self, guard: &mut Guard) -> io::Result<()> {
        if self.terminal.is_some() {
            terminal_change(|| {
                // SAFETY: terminal admission checked stdin; Guard owns the PGID.
                if unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, guard.pid()) } < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            })?;
            // A fast stdin reader can encounter SIGTTIN before the handoff.
            guard.forward(libc::SIGCONT)?;
        }
        Ok(())
    }

    pub(super) fn observe(&self, guard: &mut Guard) -> Option<StopReason> {
        let mut cancelled = false;
        for signal in observed(SignalWatch::take()) {
            if let Err(error) = guard.forward(signal) {
                return Some(StopReason::Io(error));
            }
            cancelled |= matches!(
                signal,
                libc::SIGHUP | libc::SIGINT | libc::SIGQUIT | libc::SIGTERM
            );
        }
        // Unlike an interactive session, stop/continue never stops this
        // supervisor: even a stopped child remains subject to the deadline.
        // Cancellation forwards the original signal, then uses the ordinary
        // bounded group cleanup, including when descendants ignore it.
        cancelled.then_some(StopReason::Cancelled)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(terminal) = &self.terminal {
            let _ = terminal_change(|| {
                // SAFETY: saved foreground and attributes came from this tty.
                if unsafe { libc::tcsetpgrp(libc::STDIN_FILENO, terminal.foreground) } < 0 {
                    return Err(io::Error::last_os_error());
                }
                // Never drain terminal output here: it may be flow-controlled
                // forever and must not defeat the task's finite budget.
                if unsafe {
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &terminal.attributes)
                } < 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

struct Terminal {
    foreground: libc::pid_t,
    attributes: libc::termios,
}

impl Terminal {
    fn claim() -> io::Result<Option<Self>> {
        // SAFETY: descriptor-only inquiry; a nonterminal needs no handoff.
        if unsafe { libc::isatty(libc::STDIN_FILENO) } != 1 {
            return Ok(None);
        }
        let foreground = unsafe { libc::tcgetpgrp(libc::STDIN_FILENO) };
        if foreground < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: tcgetattr initializes the owned output object.
        let mut attributes = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(libc::STDIN_FILENO, &mut attributes) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(Self {
            foreground,
            attributes,
        }))
    }
}

/// Terminal ownership changes are legal from a background process only with
/// SIGTTOU blocked. Restore exactly this thread's prior signal mask afterwards.
fn terminal_change(change: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut previous: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: initialized local signal sets; modifies only the calling thread.
    unsafe {
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTTOU);
    }
    let code = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut previous) };
    if code != 0 {
        return Err(io::Error::from_raw_os_error(code));
    }
    let result = change();
    let code = unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut()) };
    result.and_then(|()| {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(code))
        }
    })
}
