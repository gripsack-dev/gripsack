//! Activation adapters (0001 §3.8): intents run after the flip, in
//! trigger order. SystemdUser first — a service intent means the unit
//! file was deployed by the module's config entries; the adapter
//! reloads systemd and (re)starts it.
//!
//! Failure policy (0001 §3.8, hard rule): post-activation failures
//! are warnings in the report, never apply errors, never rollbacks.

use crate::report::{ReportKind, StepReport};
use gripsack_ir::Action;
use gripsack_ir::step::{Step, StepAction};

/// Step-form intents from the EXPANDED steps (declarative `activate`
/// fields expand into these — the IR's module.steps field is empty
/// for declarative modules, so this MUST be the expand output). No
/// trigger on the step form: kind routes the adapter phase (caches
/// post-link, service/custom post-activate).
fn step_intents(steps: &[Step]) -> Vec<&Action> {
    steps
        .iter()
        .filter_map(|s| match &s.action {
            StepAction::Intent { action } => Some(action.as_ref()),
            _ => None,
        })
        .collect()
}
use std::collections::BTreeMap;
use tracing::{info, warn};

/// Run every PostLink intent (fonts / desktop-entry cache refreshes),
/// deduped across modules — three font modules mean ONE fc-cache,
/// not three. Runs before PostActivate (trigger order, 0001 §3.8).
pub(crate) fn run_post_link(
    order: &[String],
    steps_by_module: &BTreeMap<String, Vec<Step>>,
) -> Vec<StepReport> {
    let mut want_fonts = false;
    let mut want_desktop = false;
    for name in order {
        // step-form intents — cache kinds are post-link (declarative
        // activate fields expand into these too, so this is the ONLY
        // place they execute)
        for action in step_intents(&steps_by_module[name.as_str()]) {
            match action {
                Action::Fonts => want_fonts = true,
                Action::DesktopEntry => want_desktop = true,
                _ => {}
            }
        }
    }
    let mut reports = Vec::new();
    if want_fonts {
        reports.push(refresh_cache(
            "fonts",
            "fc-cache",
            &["-f"],
            "fontconfig cache refreshed",
        ));
    }
    if want_desktop {
        let apps = desktop_applications_dir();
        let apps_str = apps.to_string_lossy().into_owned();
        reports.push(refresh_cache(
            "desktop-entry",
            "update-desktop-database",
            &[&apps_str],
            "desktop database refreshed",
        ));
    }
    reports
}

fn desktop_applications_dir() -> std::path::PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return std::path::PathBuf::from(xdg).join("applications");
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home).join(".local/share/applications");
    }
    std::path::PathBuf::from("~/.local/share/applications")
}

/// A cache-refresh adapter: run the tool if it exists, warn-skip
/// otherwise. Failures are warnings, never apply errors (the hard rule).
fn refresh_cache(kind: &str, tool: &str, args: &[&str], ok_msg: &str) -> StepReport {
    let available = std::process::Command::new(tool)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !available {
        return StepReport {
            module: kind.to_string(),
            summary: format!("{kind} skipped (no {tool})"),
            kind: ReportKind::Warned,
        };
    }
    let status = std::process::Command::new(tool).args(args).status();
    match status {
        Ok(s) if s.success() => StepReport {
            module: kind.to_string(),
            summary: format!("{ok_msg} ({tool})"),
            kind: ReportKind::Configured,
        },
        _ => {
            warn!(tool, "cache refresh failed");
            StepReport {
                module: kind.to_string(),
                summary: format!("{kind} refresh failed ({tool})"),
                kind: ReportKind::Warned,
            }
        }
    }
}

/// Run every PostActivate intent of the modules that applied, in
/// graph order. Returns the reports for the CLI.
pub(crate) fn run_post_activate(
    order: &[String],
    steps_by_module: &BTreeMap<String, Vec<Step>>,
) -> Vec<StepReport> {
    let mut reports = Vec::new();
    for name in order {
        // step-form intents — service/custom are post-activate
        // (single execution path, see run_post_link)
        for action in step_intents(&steps_by_module[name.as_str()]) {
            match action {
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
