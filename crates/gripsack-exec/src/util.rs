//! Small shared helpers.

use crate::ctx::Ctx;
use std::io;
use std::path::{Path, PathBuf};

/// An exclusive flock on `$GRIPSACK_HOME/locks/<name>.flock` — held
/// for one step's duration (0007 §4), dropped on scope exit. Two
/// concurrent `grip` runs serialize on the same file. The primitive
/// lives in gripsack-store (fs::FlockGuard) — one implementation for
/// apply, trust, and tool provisioning.
pub use gripsack_fs::FlockGuard;

/// One serialized mutation lifecycle (0045 F4): owns the
/// `$GRIPSACK_HOME/locks/apply.flock` guard AND the home it locks.
/// Public mutation APIs (`gc`, `rollback_generation`,
/// `verify_store`) take `&LifecycleSession` instead of a bare home,
/// so the lock can no longer be a caller convention — and a session
/// for home A can never authorize a mutation in home B. `apply` and
/// `update` acquire a session internally (their public shape is
/// unchanged). There is deliberately no "assume locked" constructor.
pub struct LifecycleSession {
    _guard: FlockGuard,
    home: PathBuf,
}

impl LifecycleSession {
    /// Take the lifecycle lock for `home`; blocks while another
    /// gripsack process holds it.
    pub fn acquire(home: &Path) -> io::Result<Self> {
        let guard = gripsack_fs::FlockGuard::acquire(&home.join("locks"), "apply")?;
        Ok(Self {
            _guard: guard,
            home: home.to_path_buf(),
        })
    }

    /// The home THIS session's lock serializes — the only home its
    /// mutation authority covers.
    pub fn home(&self) -> &Path {
        &self.home
    }
}

/// Test-only kill switch (plan/0025 acceptance):
/// GRIPSACK_CRASH_AFTER=<phase> aborts the process right after the
/// named phase, so e2e can exercise the crash windows the journal
/// closes. abort() skips destructors like a kill -9; the OS releases
/// the flocks. Never set outside tests.
pub fn crash_hook(phase: &str) {
    if std::env::var("GRIPSACK_CRASH_AFTER").as_deref() == Ok(phase) {
        std::process::abort();
    }
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
