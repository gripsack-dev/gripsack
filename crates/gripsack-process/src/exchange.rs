use super::lifecycle::Guard;
use super::{Control, Limits, StopReason, sys};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use std::time::{Duration, Instant};

const CHUNK: usize = 8192;

pub(super) struct Exchange<'a> {
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
    input: &'a [u8],
    sent: usize,
    limits: Limits,
    line: Vec<u8>,
    line_len: usize,
    responded: bool,
    out_total: u64,
    err_total: u64,
    tail: VecDeque<u8>,
}

impl<'a> Exchange<'a> {
    pub fn new(child: &mut Child, input: &'a [u8], limits: Limits) -> Self {
        Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            input,
            sent: 0,
            limits,
            line: Vec::new(),
            line_len: 0,
            responded: false,
            out_total: 0,
            err_total: 0,
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
        if self.input.is_empty() {
            self.close_input();
        }
        Ok(())
    }

    pub fn close_input(&mut self) {
        self.stdin = None;
    }
    pub fn into_tail(self) -> Vec<u8> {
        self.tail.into_iter().collect()
    }

    pub fn drive(
        &mut self,
        guard: &mut Guard,
        active_end: Instant,
        callback: &mut impl FnMut(&[u8]) -> Control,
    ) -> StopReason {
        loop {
            if Instant::now() >= active_end {
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
            let remaining = active_end.saturating_duration_since(Instant::now());
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
        let end = self.input.len().min(self.sent.saturating_add(CHUNK));
        let result = sys::without_sigpipe(|| pipe.write(&self.input[self.sent..end]));
        match result {
            Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "child stdin")),
            Ok(n) => self.sent += n,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => self.close_input(),
            Err(e) if transient(&e) => {}
            Err(e) => return Err(e),
        }
        if self.sent == self.input.len() {
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
        let mut buf = [0; CHUNK];
        let n = match pipe.read(&mut buf) {
            Ok(n) => n,
            Err(e) if transient(&e) => return Ok(()),
            Err(e) => return Err(StopReason::Io(e)),
        };
        if n == 0 {
            self.stdout = None;
            if self.line_len != 0 && !self.responded {
                self.responded = callback(&self.line) == Control::Response;
            }
            self.line.clear();
            self.line_len = 0;
            return Ok(());
        }
        count(&mut self.out_total, n, self.limits.stdout_bytes)
            .map_err(|()| StopReason::StdoutLimit)?;
        for &byte in &buf[..n] {
            if byte == b'\n' {
                if !self.responded {
                    self.responded = callback(&self.line) == Control::Response;
                }
                self.line.clear();
                self.line_len = 0;
            } else {
                if self.line_len == self.limits.line_bytes {
                    return Err(StopReason::LineLimit);
                }
                self.line_len += 1;
                if !self.responded {
                    self.line.push(byte);
                }
            }
        }
        Ok(())
    }

    fn read_stderr(&mut self) -> Result<(), StopReason> {
        let Some(pipe) = &mut self.stderr else {
            return Ok(());
        };
        let mut buf = [0; CHUNK];
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
        count(&mut self.err_total, n, self.limits.stderr_bytes)
            .map_err(|()| StopReason::StderrLimit)
    }

    fn retain(&mut self, bytes: &[u8]) {
        let cap = self.limits.retained_stderr_bytes;
        if cap == 0 {
            return;
        }
        for &byte in bytes {
            if self.tail.len() == cap {
                self.tail.pop_front();
            }
            self.tail.push_back(byte);
        }
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

fn count(total: &mut u64, n: usize, cap: u64) -> Result<(), ()> {
    let next = total.checked_add(n as u64).ok_or(())?;
    *total = next;
    if next > cap { Err(()) } else { Ok(()) }
}
