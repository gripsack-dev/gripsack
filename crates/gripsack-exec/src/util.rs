//! Small shared helpers.

use crate::ctx::Ctx;
use std::path::PathBuf;

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
