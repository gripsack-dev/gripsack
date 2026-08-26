//! Command implementations, one file per command group. Shared helpers:
//! `eval` (frontend wiring) and `expand_home`.

pub mod apply;
pub mod check;
pub mod doctor;
pub mod eval;
pub mod gc;
pub mod generations;
pub mod init;
pub mod plan;
pub mod repo;
pub mod rollback;
pub mod update;
pub mod why_owns;

pub use apply::apply;
pub use check::check;
pub use doctor::doctor;
pub use eval::{check_ir, eval_repo, run_lints, validate_sources};
pub use gc::gc;
pub use generations::generations;
pub use init::init;
pub use plan::{plan_ir, plan_module};
pub use repo::resolve as resolve_repo;
pub use rollback::rollback;
pub use update::update;
pub use why_owns::why_owns;

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
