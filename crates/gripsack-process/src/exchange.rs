use super::lifecycle::{ChildPipes, Guard};
use super::{Control, Limits, OutputMode, StopReason, sys};
use gripsack_policy::process_budget::{
    FrameAction, FrameBudget, IO_CHUNK_BYTES, InputTransfer, RetainedStderrLimit, StderrBudget,
    StdoutBudget, retain_tail,
};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{ChildStderr, ChildStdin, ChildStdout};
use std::time::Duration;

pub(super) struct Exchange<'a> {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    input: &'a [u8],
    input_transfer: InputTransfer,
    retained_stderr: RetainedStderrLimit,
    output: OutputMode,
    line: Vec<u8>,
    frame: FrameBudget,
    responded: bool,
    stdout_budget: StdoutBudget,
    stderr_budget: StderrBudget,
    tail: VecDeque<u8>,
}

impl<'a> Exchange<'a> {
    pub fn new(
        pipes: ChildPipes,
        input: &'a [u8],
        input_transfer: InputTransfer,
        limits: Limits,
        output: OutputMode,
    ) -> Self {
        Self {
            stdin: pipes.stdin,
            stdout: pipes.stdout,
            stderr: pipes.stderr,
            input,
            input_transfer,
            retained_stderr: limits.retained_stderr_bytes,
            output,
            line: Vec::new(),
            frame: FrameBudget::new(limits.line_bytes),
            responded: false,
            stdout_budget: StdoutBudget::new(limits.stdout_bytes),
            stderr_budget: StderrBudget::new(limits.stderr_bytes),
            tail: VecDeque::new(),
        }
    }

    pub fn configure(&mut self) -> io::Result<()> {
        let fds = [
            self.stdin.as_ref().map(AsRawFd::as_raw_fd),
            self.stdout.as_ref().map(AsRawFd::as_raw_fd),
            self.stderr.as_ref().map(AsRawFd::as_raw_fd),
        ];
        for fd in fds.into_iter().flatten() {
            sys::nonblocking(fd)?;
        }
        if self.input_transfer.is_closed() {
            self.close_input();
        }
        Ok(())
    }

    pub fn close_input(&mut self) {
        self.stdin = None;
        self.input_transfer.close();
    }
    pub fn into_tail(self) -> (Vec<u8>, bool) {
        let truncated = self.stderr_budget.truncated(self.retained_stderr);
        (self.tail.into_iter().collect(), truncated)
    }

    pub fn drive(
        &mut self,
        guard: &mut Guard,
        mut deadline: super::OperationDeadline,
        callback: &mut impl FnMut(&[u8]) -> Control,
    ) -> StopReason {
        loop {
            if deadline.remaining().is_none() {
                return StopReason::Deadline;
            }
            match guard.observe() {
                Ok(true) => {
                    // Kill while the zombie still pins the PGID. Drain buffered
                    // data through normal framing/accounting before declaring exit.
                    if let Err(error) = guard.terminate() {
                        return StopReason::Io(error);
                    }
                    self.close_input();
                    if self.stdout.is_none() && self.stderr.is_none() {
                        return StopReason::Exited;
                    }
                }
                Ok(false) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return StopReason::Io(error),
            }
            let Some(remaining) = deadline.remaining() else {
                return StopReason::Deadline;
            };
            if let Err(reason) = self.step(remaining, callback) {
                return reason;
            }
        }
    }

    fn step(
        &mut self,
        remaining: Duration,
        callback: &mut impl FnMut(&[u8]) -> Control,
    ) -> Result<(), StopReason> {
        let mut fds = [
            sys::pollfd(self.stdin.as_ref().map(AsRawFd::as_raw_fd), libc::POLLOUT),
            sys::pollfd(self.stdout.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
            sys::pollfd(self.stderr.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
        ];
        sys::poll(&mut fds, remaining).map_err(StopReason::Io)?;
        for fd in &fds {
            if fd.revents & libc::POLLNVAL != 0 {
                return Err(StopReason::Io(io::Error::from_raw_os_error(libc::EBADF)));
            }
        }
        // At most one chunk per direction per tick: floods cannot starve the
        // other direction, waitid or deadline checks.
        if fds[0].revents != 0 {
            self.write_input().map_err(StopReason::Io)?;
        }
        if fds[1].revents != 0 {
            self.read_stdout(callback)?;
        }
        if fds[2].revents != 0 {
            self.read_stderr()?;
        }
        Ok(())
    }

    fn write_input(&mut self) -> io::Result<()> {
        let Some(pipe) = &mut self.stdin else {
            return Ok(());
        };
        let Some(chunk) = self.input_transfer.begin() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "input write outside admitted transfer",
            ));
        };
        let result = sys::without_sigpipe(|| pipe.write(&self.input[chunk.range()]));
        match result {
            Ok(n) => {
                if !self.input_transfer.complete(n) {
                    return Err(io::Error::new(
                        if n == 0 {
                            io::ErrorKind::WriteZero
                        } else {
                            io::ErrorKind::InvalidData
                        },
                        "invalid child stdin write count",
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => self.close_input(),
            Err(e) if transient(&e) => self.input_transfer.retry(),
            Err(e) => return Err(e),
        }
        if self.input_transfer.is_closed() {
            self.close_input();
        }
        Ok(())
    }

    fn read_stdout(
        &mut self,
        callback: &mut impl FnMut(&[u8]) -> Control,
    ) -> Result<(), StopReason> {
        let Some(pipe) = &mut self.stdout else {
            return Ok(());
        };
        let mut buf = [0; IO_CHUNK_BYTES];
        let n = match pipe.read(&mut buf) {
            Ok(n) => n,
            Err(e) if transient(&e) => return Ok(()),
            Err(e) => return Err(StopReason::Io(e)),
        };
        if n == 0 {
            self.stdout = None;
            if self.frame.finish() && !self.responded {
                self.responded = callback(&self.line) == Control::Response;
            }
            self.line.clear();
            return Ok(());
        }
        if !self.stdout_budget.observe(n as u64) {
            return Err(StopReason::StdoutLimit);
        }
        if matches!(self.output, OutputMode::Raw) {
            if !self.responded {
                self.responded = callback(&buf[..n]) == Control::Response;
            }
            return Ok(());
        }
        for &byte in &buf[..n] {
            match self.frame.observe(byte) {
                FrameAction::Boundary => {
                    if !self.responded {
                        self.responded = callback(&self.line) == Control::Response;
                    }
                    self.line.clear();
                }
                FrameAction::Append => {
                    if !self.responded {
                        self.line.push(byte);
                    }
                }
                FrameAction::Limit => return Err(StopReason::LineLimit),
            }
        }
        Ok(())
    }

    fn read_stderr(&mut self) -> Result<(), StopReason> {
        let Some(pipe) = &mut self.stderr else {
            return Ok(());
        };
        let mut buf = [0; IO_CHUNK_BYTES];
        let n = match pipe.read(&mut buf) {
            Ok(n) => n,
            Err(e) if transient(&e) => return Ok(()),
            Err(e) => return Err(StopReason::Io(e)),
        };
        if n == 0 {
            self.stderr = None;
            return Ok(());
        }
        self.retain(&buf[..n]);
        if self.stderr_budget.observe(n as u64) {
            Ok(())
        } else {
            Err(StopReason::StderrLimit)
        }
    }

    fn retain(&mut self, bytes: &[u8]) {
        let append = retain_tail(self.retained_stderr, self.tail.len(), bytes.len());
        drop(self.tail.drain(..append.discard_bytes()));
        self.tail.extend(&bytes[append.skip_bytes()..]);
    }

    /// After a stop, drain without callbacks. Accounting remains active; the
    /// first stop is preserved. Cap/line violations during this phase need no
    /// additional action: the group is already being terminated. Close that
    /// stream on any violation, rather than spending unlimited work on it.
    pub fn drain(&mut self, remaining: Duration) -> io::Result<bool> {
        let mut fds = [
            sys::pollfd(self.stdout.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
            sys::pollfd(self.stderr.as_ref().map(AsRawFd::as_raw_fd), libc::POLLIN),
        ];
        if self.stdout.is_none() && self.stderr.is_none() {
            return Ok(true);
        }
        sys::poll(&mut fds, remaining)?;
        self.responded = true;
        self.line.clear();
        for (index, fd) in fds.iter().enumerate() {
            if fd.revents == 0 {
                continue;
            }
            if fd.revents & libc::POLLNVAL != 0 {
                return Err(io::Error::from_raw_os_error(libc::EBADF));
            }
            let result = if index == 0 {
                self.read_stdout(&mut |_| Control::Response)
            } else {
                self.read_stderr()
            };
            match result {
                Err(StopReason::Io(error)) => return Err(error),
                Err(_) if index == 0 => self.stdout = None,
                Err(_) => self.stderr = None,
                Ok(()) => {}
            }
        }
        Ok(self.stdout.is_none() && self.stderr.is_none())
    }
}

fn transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
}
