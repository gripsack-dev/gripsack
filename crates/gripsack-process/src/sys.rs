//! Small audited Unix boundary. EINTR from lifecycle operations is returned
//! to the deadline loop; read/write EINTR/EAGAIN are handled by Exchange.
use std::io;
use std::os::fd::RawFd;
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::time::Duration;

pub(super) fn nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: caller owns a live pipe descriptor; F_GETFL has no pointer argument.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: preserves existing flags, changes only nonblocking behavior.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn pollfd(fd: Option<RawFd>, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd: fd.unwrap_or(-1),
        events,
        revents: 0,
    }
}

pub(super) fn poll(fds: &mut [libc::pollfd], remaining: Duration) -> io::Result<()> {
    // Round down to avoid exceeding the budget merely due to millisecond
    // rounding; the final sub-ms interval may briefly spin. Tick waitid often.
    let ms = remaining.as_millis().min(10) as libc::c_int;
    // SAFETY: slice is writable for exactly nfds entries (zero is valid).
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
    if rc < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
        for fd in fds {
            fd.revents = 0;
        }
    }
    Ok(())
}

pub(super) fn observe(pid: libc::pid_t) -> io::Result<bool> {
    // SAFETY: all-zero siginfo is a valid output buffer; zero si_pid denotes
    // no event even on platforms that do not initialize it for WNOHANG.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: exclusive child PID ownership is maintained by Guard. WNOWAIT
    // observes only; it cannot release the PID for reuse.
    let rc = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: waitid initialized the siginfo; libc provides this accessor on
    // both Linux and macOS (the field layout is platform-specific).
    Ok(unsafe { info.si_pid() } == pid)
}

pub(super) fn kill_group(pid: libc::pid_t) -> io::Result<()> {
    if pid <= 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid child pgid",
        ));
    }
    // SAFETY: Guard has observed and still owns the unreaped process-group
    // leader; this is never the parent's group or an arbitrary supplied PID.
    if unsafe { libc::killpg(pid, libc::SIGKILL) } < 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn reap(pid: libc::pid_t) -> io::Result<Option<ExitStatus>> {
    let mut raw = 0;
    // SAFETY: valid status pointer, exact owned child PID, never blocking.
    let rc = unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(if rc == 0 {
        None
    } else {
        Some(ExitStatus::from_raw(raw))
    })
}

/// Block SIGPIPE only on this thread around a pipe write. Do not alter the
/// process-wide disposition or steal a pre-existing pending SIGPIPE. Unlike
/// sigtimedwait, sigwait is available on both supported platforms.
pub(super) fn without_sigpipe(write: impl FnOnce() -> io::Result<usize>) -> io::Result<usize> {
    // SAFETY: initialized by sigemptyset before being passed as signal sets.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    let mut old: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        // SAFETY: pointers refer to valid local sigset_t objects.
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGPIPE);
    }
    // SAFETY: modifies only the calling thread's mask, saving its exact value.
    let rc = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, &mut old) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    let mut restore = Mask { old, active: true };
    let before = pending_pipe()?;
    let result = write();
    if matches!(&result, Err(e) if e.kind() == io::ErrorKind::BrokenPipe)
        && !before
        && pending_pipe()?
    {
        let mut signal = 0;
        // SAFETY: SIGPIPE is blocked and known pending on this thread after
        // EPIPE, so this consumes it without waiting for a future signal.
        // The caller must not concurrently consume this thread's SIGPIPE.
        let rc = unsafe { libc::sigwait(&set, &mut signal) };
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
    }
    restore.restore()?;
    result
}

fn pending_pipe() -> io::Result<bool> {
    // SAFETY: output-only initialized signal set and a valid signal number.
    let mut pending: libc::sigset_t = unsafe { std::mem::zeroed() };
    if unsafe { libc::sigpending(&mut pending) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { libc::sigismember(&pending, libc::SIGPIPE) } == 1)
}

struct Mask {
    old: libc::sigset_t,
    active: bool,
}
impl Mask {
    fn restore(&mut self) -> io::Result<()> {
        // SAFETY: old was obtained from pthread_sigmask on this same thread.
        let rc =
            unsafe { libc::pthread_sigmask(libc::SIG_SETMASK, &self.old, std::ptr::null_mut()) };
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc));
        }
        self.active = false;
        Ok(())
    }
}
impl Drop for Mask {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}
