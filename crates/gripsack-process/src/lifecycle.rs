use super::sys;
use gripsack_policy::process_budget::{
    ChildLifecycle, CleanupDecision, ObserveAction, ReapObservation, SignalAction,
};
use std::io;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, ExitStatus};
use std::time::{Duration, Instant};

pub(super) struct ChildPipes {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

/// Owns the exclusive right to observe/reap the child and signal its group.
/// No signal is issued after waitpid, or after loss of wait ownership.
pub(super) struct Guard {
    pid: libc::pid_t,
    deadline: super::OperationDeadline,
    state: ChildLifecycle,
    signal_error: Option<io::Error>,
}

impl Guard {
    pub fn new(mut child: Child, deadline: Instant) -> (Self, ChildPipes) {
        let guard = Self {
            pid: child.id() as libc::pid_t,
            deadline: super::OperationDeadline::at(deadline),
            state: ChildLifecycle::new(),
            signal_error: None,
        };
        let pipes = ChildPipes {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
        };
        (guard, pipes)
    }

    pub fn observe(&mut self) -> io::Result<bool> {
        match self.state.observe_action() {
            ObserveAction::Unavailable => {
                return Err(io::Error::from_raw_os_error(libc::ECHILD));
            }
            ObserveAction::Exited => return Ok(true),
            ObserveAction::Observe => {}
        }
        match sys::observe(self.pid) {
            Ok(exited) => {
                self.state.observe_exit(exited);
                Ok(exited)
            }
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.state.lose_ownership();
                }
                Err(error)
            }
        }
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        // Observe even a running leader before signalling. ECHILD means the
        // ownership contract was violated: never risk signalling a reused PID.
        self.observe()?;
        match self.state.signal_action() {
            SignalAction::AlreadySignalled => return Ok(()),
            SignalAction::Unavailable => {
                return Err(io::Error::from_raw_os_error(libc::ECHILD));
            }
            SignalAction::Signal => {}
        }
        match sys::kill_group(self.pid, self.deadline.instant()) {
            Ok(()) => {
                self.state.signal_succeeded();
                Ok(())
            }
            Err(error) => {
                if error.kind() != io::ErrorKind::Interrupted && self.signal_error.is_none() {
                    self.signal_error = Some(match error.raw_os_error() {
                        Some(code) => io::Error::from_raw_os_error(code),
                        None => io::Error::new(error.kind(), error.to_string()),
                    });
                }
                Err(error)
            }
        }
    }

    fn reap(&mut self) -> io::Result<Option<ExitStatus>> {
        if !self.state.begin_reap() {
            return Err(io::Error::from_raw_os_error(libc::ECHILD));
        }
        let result = sys::reap(self.pid);
        let observation = match &result {
            Ok(Some(_)) => ReapObservation::Reaped,
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => ReapObservation::Lost,
            _ => ReapObservation::NotReady,
        };
        self.state.observe_reap(observation);
        result
    }

    pub fn finish(
        &mut self,
        mut drain: impl FnMut(Duration) -> io::Result<bool>,
    ) -> (Option<ExitStatus>, Option<io::Error>) {
        let mut error = self.signal_error.take();
        let mut status = None;
        let mut drained = false;
        loop {
            if self.state.needs_termination()
                && let Err(e) = self.terminate()
                && e.kind() != io::ErrorKind::Interrupted
            {
                record(&mut error, e);
            }
            // Reap once signalling succeeded OR the leader is observed exited.
            // A real signal failure is retained, but must not strand a zombie.
            if self.state.should_observe_for_reap() {
                match self.observe() {
                    Ok(true) => match self.reap() {
                        Ok(Some(s)) => {
                            status = Some(s);
                        }
                        Ok(None) => {}
                        Err(e) => {
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
            let remaining = self.deadline.remaining().unwrap_or_default();
            if !drained {
                match drain(remaining) {
                    Ok(done) => drained = done,
                    Err(e) => {
                        record(&mut error, e);
                        drained = true;
                    }
                }
            }
            match self
                .state
                .cleanup_decision(drained, self.deadline.remaining().is_some())
            {
                CleanupDecision::Complete => break,
                CleanupDecision::LostOwnership => {
                    record(&mut error, io::Error::from_raw_os_error(libc::ECHILD));
                    break;
                }
                CleanupDecision::Deadline => {
                    record(
                        &mut error,
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "cleanup budget exhausted before completion",
                        ),
                    );
                    break;
                }
                CleanupDecision::Wait => {}
            }
            // drain normally supplies the poll sleep. When pipes are gone,
            // use a bounded empty poll instead of spinning on a running leader.
            if drained {
                let _ = sys::poll(&mut [], self.deadline.remaining().unwrap_or_default());
            }
        }
        if status.is_none() && error.is_none() {
            record(&mut error, io::Error::from_raw_os_error(libc::ECHILD));
        }
        if let Some(e) = self.signal_error.take() {
            record(&mut error, e);
        }
        self.state.finish();
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
        if self.state.is_finished() {
            return;
        }
        // No callbacks, blocking waits or extra grace period during unwinding.
        // Errors cannot be returned from Drop. The normal path reports them.
        let _ = self.finish(|_| Ok(true));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    #[test]
    fn reaped_leader_cannot_reauthorize_group_signals() {
        let child = Command::new("/bin/sh")
            .args(["-c", ":"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .unwrap();
        let end = Instant::now() + Duration::from_secs(2);
        let (mut guard, _pipes) = Guard::new(child, end);
        while !guard.observe().unwrap() {
            assert!(Instant::now() < end, "child did not exit");
            std::thread::yield_now();
        }
        guard.terminate().unwrap();
        assert!(guard.reap().unwrap().unwrap().success());
        assert_eq!(
            guard
                .terminate()
                .expect_err("reaped_leader_regained_signal_authority")
                .raw_os_error(),
            Some(libc::ECHILD),
            "reaped_leader_regained_signal_authority",
        );
    }
}
