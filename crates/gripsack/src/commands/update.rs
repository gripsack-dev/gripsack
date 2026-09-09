use crate::commands::{check_ir, eval_repo, trust_gate};
use crate::render::Palette;
use gripsack_exec::{Ctx, UpdateCheckOutcome, UpdateMode, UpdateStatus, UpdateSummary};
use gripsack_store as store;
use owo_colors::OwoColorize;
use std::path::Path;
use std::process::ExitCode;

/// Survey failures are data; setup failures abort with the distinct error exit.
pub fn update(
    repo: &Path,
    host: Option<&str>,
    modules: Vec<String>,
    palette: Palette,
    check: bool,
) -> ExitCode {
    let failed = if check {
        ExitCode::from(2)
    } else {
        ExitCode::FAILURE
    };
    if trust_gate(repo).is_some() {
        return failed;
    }
    let outcome = match eval_repo(repo, host, palette) {
        Ok(outcome) => outcome,
        Err(_) => return failed,
    };
    let ir = match check_ir(&outcome.ir_json, palette) {
        Ok(ir) => ir,
        Err(_) => return failed,
    };
    let ctx = Ctx {
        home: store::gripsack_home(),
        home_dir: Default::default(),
        repo: repo.into(),
        only: modules,
        host: outcome.host.clone(),
        on_progress: None,
        take_over: false,
        take_over_entries: None,
        jobs: None,
        fetch: std::sync::Arc::clone(&outcome.fetch),
    };
    let mode = if check {
        UpdateMode::Check
    } else {
        UpdateMode::Publish
    };
    let result = gripsack_exec::update(&ir, &ctx, mode);
    gripsack_fetch::throttle::save_global();
    let reports = match result {
        Ok(reports) => reports,
        Err(error) => {
            eprintln!("error: {error}");
            return failed;
        }
    };
    if reports.is_empty() {
        println!("nothing to resolve — no selected modules");
    }
    for report in &reports {
        let status = match &report.status {
            UpdateStatus::Unchanged => "unchanged".to_string(),
            UpdateStatus::Bumped { old, new } => format!(
                "{} ({} → {new})",
                if check { "would bump" } else { "bumped" },
                old.as_deref().unwrap_or("unlocked")
            ),
            UpdateStatus::Skipped { reason } => format!("skipped ({reason})"),
            UpdateStatus::Failed { error } => format!("failed: {error}"),
        };
        if palette.enabled {
            let status = match report.status {
                UpdateStatus::Failed { .. } => palette.error(&status),
                UpdateStatus::Bumped { .. } => palette.warn(&status),
                _ => palette.dim(&status),
            };
            println!("  {} {status}", report.module.cyan());
        } else {
            println!("  {} {status}", report.module);
        }
        if let Some(layout) = report.layout.summary() {
            println!("    {layout}");
        }
    }
    let summary = UpdateSummary::from_reports(&reports);
    if check {
        let disposition = summary.outcome();
        println!(
            "survey {}: {} unchanged, {} would change, {} skipped, {} failed — lockfile and source cache unchanged",
            if disposition == UpdateCheckOutcome::Incomplete {
                "incomplete"
            } else {
                "complete"
            },
            summary.unchanged,
            summary.changed,
            summary.skipped,
            summary.failed
        );
        match disposition {
            UpdateCheckOutcome::Current => ExitCode::SUCCESS,
            UpdateCheckOutcome::ChangesAvailable => ExitCode::from(1),
            UpdateCheckOutcome::Incomplete => ExitCode::from(2),
        }
    } else {
        if summary.changed != 0 {
            println!("lockfile updated — run `grip apply` to deploy");
        }
        ExitCode::SUCCESS
    }
}
