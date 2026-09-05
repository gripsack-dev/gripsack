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
use gripsack_store as store;

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

/// Collect every module's activation intents in trigger order
/// (0001 §3.8): post-link caches first (fonts, desktop-entry), then
/// post-activate (services, custom hooks). The list is DATA — it
/// lands in the durable pending record (0032) before the flip, and
/// resume runs from the record, not a re-read of the repo.
pub(crate) fn collect(
    order: &[String],
    steps_by_module: &BTreeMap<String, Vec<Step>>,
) -> Vec<store::activation::PendingIntent> {
    let mut caches = Vec::new();
    let mut rest = Vec::new();
    for name in order {
        for action in step_intents(&steps_by_module[name.as_str()]) {
            let intent = store::activation::PendingIntent {
                module: name.clone(),
                action: action.clone(),
            };
            match action {
                Action::Fonts | Action::DesktopEntry => caches.push(intent),
                Action::Service { .. } | Action::CustomShell { .. } => rest.push(intent),
            }
        }
    }
    caches.extend(rest);
    caches
}

/// Run the intents, deduped by kind — three font modules mean ONE
/// fc-cache, not three. Failures are warnings, never apply errors
/// (0001 §3.8, hard rule). Returns the reports for the CLI.
pub(crate) fn run(intents: &[store::activation::PendingIntent]) -> Vec<StepReport> {
    let mut reports = Vec::new();
    let mut want_fonts = false;
    let mut want_desktop = false;
    let mut rest = Vec::new();
    for intent in intents {
        match &intent.action {
            Action::Fonts => want_fonts = true,
            Action::DesktopEntry => want_desktop = true,
            other => rest.push((intent.module.as_str(), other)),
        }
    }
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
    for (module, action) in rest {
        match action {
            Action::Service { name: svc, user } => {
                reports.push(service(module, svc, *user));
            }
            Action::CustomShell { script } => {
                reports.push(custom_shell(module, script));
            }
            other => {
                info!(?other, "intent declared (adapter later)");
            }
        }
    }
    reports
}

/// The resume step (0032): every lifecycle run starts here, after
/// journal reconcile, under the lock. A pending record naming the
/// CURRENT generation re-runs its intents (they are idempotent
/// refreshes by contract); anything else names a superseded or
/// rolled-back generation — discarded, never run.
pub(crate) fn resume_pending(
    home: &gripsack_fs::Dir,
    current: Option<u64>,
) -> std::io::Result<Vec<StepReport>> {
    let Some(pending) = store::activation::read_pending(home)? else {
        return Ok(Vec::new());
    };
    if Some(pending.generation) != current {
        // the named generation never committed (or is no longer
        // current) — running its adapters now would activate state
        // that isn't live
        store::activation::clear_pending(home)?;
        return Ok(Vec::new());
    }
    let mut reports = run(&pending.intents);
    store::activation::clear_pending(home)?;
    if !pending.intents.is_empty() {
        reports.insert(
            0,
            StepReport {
                module: "*".into(),
                summary: format!(
                    "resumed activation for generation {} (interrupted run)",
                    pending.generation
                ),
                kind: ReportKind::Warned,
            },
        );
    }
    Ok(reports)
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
