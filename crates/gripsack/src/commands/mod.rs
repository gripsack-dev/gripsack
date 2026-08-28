//! Command implementations, one file per command group. Shared helpers:
//! `eval` (frontend wiring), `trust_gate` (the eval gate), and
//! `expand_home`.

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
pub mod self_update;
mod store_verify;
pub mod trust;
pub mod update;
pub mod why_owns;

pub use apply::apply;
pub use check::check;
pub use doctor::doctor;
pub use eval::{check_ir, eval_repo, render_host_inputs, run_lints, validate_sources};
pub use gc::gc;
pub use generations::generations;
pub use init::init;
pub use plan::{plan_ir, plan_module};
pub use repo::resolve as resolve_repo;
pub use rollback::rollback;
pub use store_verify::store_verify;
pub use trust::{TrustCommand, trust};
pub use update::update;
pub use why_owns::why_owns;

/// The trust gate (0013 D7): call before the first frontend eval of a
/// repo. `Some(code)` = untrusted and the user declined (or there is
/// no TTY to ask) — the message is printed here, the caller returns
/// the code.
pub fn trust_gate(repo: &Path) -> Option<ExitCode> {
    match gripsack_store::trust::ensure_trusted(repo) {
        Ok(()) => None,
        Err(e) => {
            eprintln!("{}", format!("error: {e}").red().bold());
            Some(ExitCode::FAILURE)
        }
    }
}

/// The machine's hostname — $HOSTNAME, else the `hostname` command.
/// init and eval MUST agree on this (a mismatch means init writes
/// hosts/<cmd-hostname> and eval looks for hosts/<env-hostname>).
pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|h| !h.is_empty())
        })
        .unwrap_or_else(|| "default".into())
}

/// `~/...` expands against $HOME; absolute paths pass through.
pub fn expand_home(to: &str) -> PathBuf {
    if let Some(rest) = to.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(to)
}
use owo_colors::OwoColorize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
