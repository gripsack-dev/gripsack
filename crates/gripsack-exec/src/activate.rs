//! Activation adapters (0001 §3.8): intents run after the flip, in
//! trigger order. SystemdUser first — a service intent means the unit
//! file was deployed by the module's config entries; the adapter
//! reloads systemd and (re)starts it.
//!
//! Failure policy (0001 §3.8, hard rule): post-activation failures
//! are warnings in the report, never apply errors, never rollbacks.

use crate::report::{ReportKind, StepReport};
use gripsack_ir::{Action, Module, Trigger};
use std::collections::BTreeMap;
use tracing::{info, warn};

/// Run every PostActivate intent of the modules that applied, in
/// graph order. Returns the reports for the CLI.
pub(crate) fn run_post_activate(
    order: &[String],
    modules: &BTreeMap<String, Module>,
) -> Vec<StepReport> {
    let mut reports = Vec::new();
    for name in order {
        let module = &modules[name.as_str()];
        for intent in &module.activate {
            if intent.trigger != Trigger::PostActivate {
                info!(?intent.action, "intent recorded (runs in a later phase)");
                continue;
            }
            match &intent.action {
                Action::Service { name: svc, user } => {
                    reports.push(service(name, svc, *user));
                }
                Action::CustomShell { script } => {
                    reports.push(custom_shell(name, script));
                }
                other => {
                    info!(?other, "intent declared (adapter later)");
                }
            }
        }
    }
    reports
}

fn systemctl_available(user: bool) -> bool {
    let mut cmd = std::process::Command::new("systemctl");
    if user {
        cmd.arg("--user");
    }
    cmd.arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn service(module: &str, svc: &str, user: bool) -> StepReport {
    let scope = if user { "user" } else { "system" };
    if !systemctl_available(user) {
        warn!(
            service = svc,
            "systemctl not available — service intent skipped"
        );
        return StepReport {
            module: module.to_string(),
            summary: format!("service {svc} skipped (no systemctl --{scope})"),
            kind: ReportKind::Warned,
        };
    }
    let run = |args: &[&str]| {
        let mut cmd = std::process::Command::new("systemctl");
        if user {
            cmd.arg("--user");
        }
        cmd.args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
    };
    if let Err(e) = run(&["daemon-reload"]) {
        warn!(service = svc, "daemon-reload failed: {e}");
    }
    match run(&["enable", "--now", svc]) {
        Ok(out) if out.status.success() => StepReport {
            module: module.to_string(),
            summary: format!("service {svc} enabled ({scope})"),
            kind: ReportKind::Configured,
        },
        Ok(out) => {
            let tail = String::from_utf8_lossy(&out.stderr);
            warn!(service = svc, "enable --now failed: {tail}");
            StepReport {
                module: module.to_string(),
                summary: format!("service {svc} failed to enable ({scope})"),
                kind: ReportKind::Warned,
            }
        }
        Err(e) => {
            warn!(service = svc, "enable --now spawn failed: {e}");
            StepReport {
                module: module.to_string(),
                summary: format!("service {svc} failed to enable ({scope})"),
                kind: ReportKind::Warned,
            }
        }
    }
}

fn custom_shell(module: &str, script: &str) -> StepReport {
    let dir = std::env::temp_dir();
    match crate::verify::run_shell(script, &dir) {
        Ok(()) => StepReport {
            module: module.to_string(),
            summary: "custom hook ran".to_string(),
            kind: ReportKind::Configured,
        },
        Err(detail) => {
            warn!(%detail, "custom hook failed");
            StepReport {
                module: module.to_string(),
                summary: format!("custom hook failed: {detail}"),
                kind: ReportKind::Warned,
            }
        }
    }
}
