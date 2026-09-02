//! Small shared helpers.

use crate::ctx::Ctx;
use std::io;
use std::path::{Path, PathBuf};

/// An exclusive flock on `$GRIPSACK_HOME/locks/<name>.flock` — held
/// for one step's duration (0007 §4), dropped on scope exit. Two
/// concurrent `grip` runs serialize on the same file. The primitive
/// lives in gripsack-store (fs::FlockGuard) — one implementation for
/// apply, trust, and tool provisioning.
pub use gripsack_store::fs::FlockGuard;

/// Hold the apply lifecycle lock (`apply.flock`) — the public handle
/// for apply and rollback.
pub fn acquire_lifecycle_lock(home: &Path) -> io::Result<FlockGuard> {
    gripsack_store::fs::FlockGuard::acquire(&home.join("locks"), "apply")
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
