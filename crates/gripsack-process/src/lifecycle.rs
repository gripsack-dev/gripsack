use super::sys;
use std::io;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

/// Owns the exclusive right to observe/reap the child and signal its group.
/// No signal is issued after waitpid, or after loss of wait ownership.
pub(super) struct Guard {
    pid: libc::pid_t,
    end: Instant,
    owned: bool,
    exited: bool,
    killed: bool,
    finished: bool,
    signal_error: Option<io::Error>,
}

impl Guard {
    pub fn new(pid: u32, end: Instant) -> Self {
        Self {
            pid: pid as libc::pid_t,
            end,
            owned: true,
            exited: false,
            killed: false,
            finished: false,
            signal_error: None,
        }
    }

    pub fn observe(&mut self) -> io::Result<bool> {
        if !self.owned {
            return Err(io::Error::from_raw_os_error(libc::ECHILD));
        }
        if self.exited {
            return Ok(true);
        }
        match sys::observe(self.pid) {
            Ok(exited) => {
                self.exited = exited;
                Ok(exited)
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.owned = false;
                }
                Err(error)
            }
        }
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        if self.killed {
            return Ok(());
        }
        // Observe even a running leader before signalling. ECHILD means the
        // ownership contract was violated: never risk signalling a reused PID.
        self.observe()?;
        match sys::kill_group(self.pid) {
            Ok(()) => {
                self.killed = true;
                Ok(())
            }
            Err(error) => {
                if error.kind() != io::ErrorKind::Interrupted && self.signal_error.is_none() {
                    self.signal_error = Some(io::Error::new(error.kind(), error.to_string()));
                }
                Err(error)
            }
        }
    }

    pub fn finish(
        &mut self,
        mut drain: impl FnMut(Duration) -> io::Result<bool>,
    ) -> (Option<ExitStatus>, Option<io::Error>) {
        let mut error = self.signal_error.take();
        let mut status = None;
        let mut drained = false;
        loop {
            if !self.killed
                && self.owned
                && let Err(e) = self.terminate()
                && e.kind() != io::ErrorKind::Interrupted
            {
                record(&mut error, e);
            }
            // No further group signals after a successful reap. Only reap
            // after termination succeeded, or ownership has already been lost.
            if self.killed && self.owned {
                match self.observe() {
                    Ok(true) => match sys::reap(self.pid) {
                        Ok(Some(s)) => {
                            status = Some(s);
                            self.owned = false;
                        }
                        Ok(None) => {}
                        Err(e) => {
                            if e.raw_os_error() == Some(libc::ECHILD) {
                                self.owned = false;
                            }
                            if e.kind() != io::ErrorKind::Interrupted {
                                record(&mut error, e);
                            }
                        }
                    },
                    Ok(false) => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => record(&mut error, e),
                }
            }
            let remaining = self.end.saturating_duration_since(Instant::now());
            if !drained {
                match drain(remaining) {
                    Ok(done) => drained = done,
                    Err(e) => {
                        record(&mut error, e);
                        drained = true;
                    }
                }
            }
            if !self.owned && drained {
                break;
            }
            if Instant::now() >= self.end {
                record(
                    &mut error,
                    io::Error::new(
                        io::ErrorKind::TimedOut,
                        "cleanup budget exhausted before leader reap and pipe EOF",
                    ),
                );
                break;
            }
            // drain normally supplies the poll sleep. When pipes are gone,
            // use a bounded empty poll instead of spinning on a running leader.
            if drained {
                let _ = sys::poll(&mut [], self.end.saturating_duration_since(Instant::now()));
            }
        }
        if status.is_none() && error.is_none() {
            record(&mut error, io::Error::from_raw_os_error(libc::ECHILD));
        }
        if let Some(e) = self.signal_error.take() {
            record(&mut error, e);
        }
        self.finished = true;
        (status, error)
    }
}

fn record(slot: &mut Option<io::Error>, error: io::Error) {
    if slot.is_none() {
        *slot = Some(error);
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // No callbacks, blocking waits or extra grace period during unwinding.
        // Errors cannot be returned from Drop. The normal path reports them.
        let _ = self.finish(|_| Ok(true));
    }
}
