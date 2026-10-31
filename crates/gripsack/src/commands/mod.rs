//! Command implementations, one file per command group. Shared helpers:
//! `eval` (frontend wiring), `trust_gate` (the eval gate), and
//! `expand_home`.

pub mod adopt;
pub mod apply;
pub mod build;
pub mod check;
pub mod doctor;
pub mod eval;
pub mod frontend;
pub mod gc;
pub mod generations;
pub mod hooks;
pub mod init;
pub mod jobs;
pub mod package;
pub mod plan;
mod prepared;
pub mod probe;
pub mod repo;
pub mod review;
pub mod rollback;
pub mod run;
pub mod saved_plan;
pub mod self_update;
pub mod services;
mod store_verify;
pub mod trust;
pub mod update;
pub mod why_owns;

pub use adopt::adopt;
pub use apply::{ApplyOptions, apply, apply_scoped};
pub use build::{BuildArgs, build};
pub use check::check;
pub use doctor::doctor;
pub use eval::{
    check_ir, eval_repo, reject_workspace_execution, render_host_inputs, validate_sources,
    validated_ir,
};
pub use gc::gc;
pub use generations::generations;
pub use hooks::{HooksCommand, hooks};
pub use init::init;
pub use plan::plan_ir;
pub use repo::resolve as resolve_repo;
pub use rollback::rollback;
pub use run::{
    PackageCommandArgs, RunArgs, ShellArgs, TaskArgs, package_command, run, shell, task,
};
pub use store_verify::store_verify;
pub use trust::{TrustCommand, trust};
pub use update::update;
pub use why_owns::why_owns;

/// The machine's hostname — $HOSTNAME, else the `hostname` command.
/// Used only as the default workspace lock identity.
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

/// Normalize an ambient hostname into a safe workspace lock identity.
pub fn sanitize_hostname(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if clean.is_empty() {
        "myhost".into()
    } else {
        clean
    }
}

/// Default workspace lock identity; never selects a source entrypoint.
pub fn default_host() -> String {
    sanitize_hostname(&hostname())
}
/// `~/...` expands against $HOME; absolute paths pass through.
pub fn expand_home(to: &str) -> PathBuf {
    gripsack_store::expand_home(to)
}
use std::path::PathBuf;
