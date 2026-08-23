//! Command implementations, one file per command group. Shared helpers:
//! `eval` (frontend wiring) and `expand_home`.

pub mod apply;
pub mod eval;
pub mod generations;
pub mod plan;
pub mod rollback;
pub mod update;

pub use apply::apply;
pub use eval::{check_ir, eval_repo};
pub use generations::generations;
pub use plan::{plan_ir, plan_module};
pub use rollback::rollback;
pub use update::update;

/// `~/...` expands against $HOME; absolute paths pass through.
pub fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(to)
}

use std::path::PathBuf;
