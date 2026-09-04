//! Command implementations, one file per command group. Shared helpers:
//! `eval` (frontend wiring), `trust_gate` (the eval gate), and
//! `expand_home`.

pub mod adopt;
pub mod apply;
pub mod check;
pub mod doctor;
pub mod eval;
pub mod frontend;
pub mod gc;
pub mod generations;
pub mod init;
pub mod plan;
pub mod probe;
pub mod repo;
pub mod rollback;
pub mod self_update;
mod store_verify;
pub mod trust;
pub mod update;
pub mod why_owns;

pub use adopt::adopt;
pub use apply::{ApplyOptions, apply, apply_scoped};
pub use check::check;
pub use doctor::doctor;
pub use eval::{check_ir, eval_repo, render_host_inputs, validate_sources, validated_ir};
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
            eprintln!("{}", Palette::detect().error(&format!("error: {e}")));
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

/// A valid host FILE name: alnum, dash, underscore — everything else
/// (macOS hostnames carry dots) becomes a dash. The single
/// sanitization for BOTH the file `grip init` writes and the default
/// host every command resolves: init wrote `foo-bar.ts` while eval
/// looked up raw `foo.bar` — `init && check` failed on every Mac
/// whose hostname has a dot (macOS CI round).
pub fn sanitize_hostname(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
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

/// The sanitized hostname — the default host everywhere (the file
/// name and the lookup agree by construction).
pub fn default_host() -> String {
    sanitize_hostname(&hostname())
}
/// `~/...` expands against $HOME; absolute paths pass through.
pub fn expand_home(to: &str) -> PathBuf {
    gripsack_store::expand_home(to)
}
use crate::render::Palette;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
