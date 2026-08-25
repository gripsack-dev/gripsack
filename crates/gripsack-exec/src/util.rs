//! Small shared helpers.

use crate::ctx::Ctx;
use std::io;
use std::path::{Path, PathBuf};

/// An exclusive flock on `$GRIPSACK_HOME/locks/<name>.flock` — held
/// for one step's duration (0007 §4), dropped on scope exit. Two
/// concurrent `grip` runs serialize on the same file.
pub struct FlockGuard(std::fs::File);

/// Hold the apply lifecycle lock (`apply.flock`) — the public handle
/// for apply and rollback.
pub fn acquire_lifecycle_lock(home: &Path) -> io::Result<FlockGuard> {
    FlockGuard::acquire(home, "apply")
}

impl FlockGuard {
    pub(crate) fn acquire(home: &Path, name: &str) -> io::Result<Self> {
        let dir = home.join("locks");
        std::fs::create_dir_all(&dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(dir.join(format!("{name}.flock")))?;
        flock(&file, true)?;
        Ok(Self(file))
    }
}

impl Drop for FlockGuard {
    fn drop(&mut self) {
        let _ = flock(&self.0, false);
    }
}

#[cfg(unix)]
fn flock(file: &std::fs::File, exclusive: bool) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let op = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_UN
    };
    if unsafe { libc::flock(file.as_raw_fd(), op) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Non-unix platforms get an error, never a silent no-op lock (N5) —
/// a lock primitive that pretends is worse than none.
#[cfg(not(unix))]
fn flock(_file: &std::fs::File, _exclusive: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "flock is not supported on this platform",
    ))
}

pub(crate) fn progress(ctx: &Ctx, module: &str, verb: &str) {
    if let Some(cb) = &ctx.on_progress {
        cb(module, verb);
    }
}

pub(crate) fn fresh_staging(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gripsack-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
